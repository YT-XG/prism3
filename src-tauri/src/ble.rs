//! 蓝牙 LE 耳机（StarRing Ultra）的**帧编解码** + **会话线程** + **自动连接看门狗**。
//!
//! 设计要点：
//! - **编解码与会话分离**：帧构造 / 解析 / 解码全是无副作用的纯函数，`#[cfg(test)]`
//!   用真机抓到的报文做逐字节断言；btleplug 只出现在会话线程里。
//! - **状态 + 广播**（照 `file_index.rs` 的模型）：`BleState` 持有 `RwLock<Inner>` +
//!   `AtomicU64` 会话代号 + 取消标志，变化时 `emit("ble:state")`；命令层另有
//!   `get_ble_state` 兜底拉取（事件不缓存）。
//! - **会话线程自带 current-thread tokio 运行时**：btleplug 是 async API，而本项目其余
//!   后台任务是裸 `std::thread`，所以运行时只在本线程内建，不引入全局运行时。
//! - **扫描策略**：耳机的广播里**既没有设备名也没有服务 UUID**，因此不能按
//!   `ScanFilter { services }` 或广播名匹配（实测两者都扫不到），只能无过滤扫描 +
//!   按地址挑选，详见 `find_peripheral`。
//! - **配置镜像**：设备列表 / 活动目标 / 自动连接开关来自 `AppConfig`（见 `config.rs`），
//!   由 `sync_from_config` 灌进 `Inner` 并随快照一起广播；连接成功后 worker 会把
//!   「最近连接时间 + 读到的设备名」写回配置并落盘（重启后仍知道该连谁）。
//! - **自动连接看门狗**：`ble_auto_connect` 开着且已知目标时才起（见 `watch_loop`），
//!   周期性**有界扫描**，扫到就把连接交给同一个会话线程；连接 / 连接中绝不扫描。
//!
//! 协议（逆向自厂商 App，见 `docs/逆向`）：
//! ```text
//! 请求 (App → 耳机):  08 EE 00 00 00 <key> <idx> <LEN> <data…> <CKSUM>
//! 应答 (耳机 → App):  09 FF 00 00 <FLG> <key> <idx> <LEN> <data…> <CKSUM>
//! ```
//! `LEN` = 整帧总长度，`CKSUM` = 前方所有字节之和 `& 0xFF`。

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use btleplug::api::{
    BDAddr, Central, CentralState, Characteristic, Manager as _, Peripheral as _,
    PeripheralProperties, ScanFilter, ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager as _};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio_stream::{Stream, StreamExt};
use uuid::Uuid;

use crate::config::{AppConfig, AppState, BleDeviceEntry};
use crate::error::{AppError, AppResult};

/// 会话状态变化（阶段 / 读数 / 错误）的广播事件名。
///
/// 广播给所有窗口：主窗口与快捷小窗都要显示同一份耳机状态。
pub const BLE_STATE_EVENT: &str = "ble:state";

/// 目标耳机地址。**地址是唯一稳定的主键**：耳机不广播设备名，也不广播服务 UUID。
pub const DEVICE_ADDRESS: &str = "12:02:03:04:7C:0F";

/// 一问一答所在的服务。
pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x011b_f5da_0000_1000_8000_0080_5f9b_34fb);
/// 写入特征（同时支持 write 与 write-without-response）。
pub const WRITE_CHARACTERISTIC_UUID: Uuid =
    Uuid::from_u128(0x0000_7777_0000_1000_8000_0080_5f9b_34fb);
/// 通知特征：耳机把请求原样回显到这里，随后才发真正的应答帧。
pub const NOTIFY_CHARACTERISTIC_UUID: Uuid =
    Uuid::from_u128(0x0000_8888_0000_1000_8000_0080_5f9b_34fb);
/// 标准 GATT Device Name 特征（`0x2A00`）。
///
/// 耳机**不广播名字**（实测 `local_name = None`），真名只在这里；它通常位于标准 GAP
/// 服务 `00001800-…` 下，但也可能被固件塞进别的服务，因此查找时**跨全部已发现服务**。
pub const DEVICE_NAME_CHARACTERISTIC_UUID: Uuid =
    Uuid::from_u128(0x0000_2a00_0000_1000_8000_0080_5f9b_34fb);

/// 广播里厂商数据的公司标识（蓝牙 SIG 分配号 `0x5452`）。
///
/// 只用于**过滤候选设备**（见 `has_vendor_manufacturer_data`），不作身份判据。
const DEVICE_COMPANY_ID: u16 = 0x5452;

/// 帧头：请求固定 `08 EE 00 00 00`。
const REQUEST_PREFIX: [u8; 5] = [0x08, 0xEE, 0x00, 0x00, 0x00];
/// 帧头：应答固定 `09 FF`，也是**回声门控**的判据。
const RESPONSE_PREFIX: [u8; 2] = [0x09, 0xFF];
/// 帧内字段偏移（请求与应答同构，故共用一套偏移）。
const KEY_OFFSET: usize = 5;
const IDX_OFFSET: usize = 6;
const LEN_OFFSET: usize = 7;
const DATA_OFFSET: usize = 8;
/// 无载荷请求携带的单个数据字节。
const EMPTY_DATA: u8 = 0x00;
/// 「不可用」哨兵：电量读数为该值时必须映射为 `None`，而不是 0 / 255。
const UNAVAILABLE: u8 = 0xFF;

/// 写标志位：`writeIdx = readIdx | WRITE_FLAG`。
const WRITE_FLAG: u8 = 0x80;
/// 应答里的 `idx` 是否清掉 `WRITE_FLAG` **并不一致**，因此匹配时一律抹掉这一位。
const IDX_MASK: u8 = 0x7F;

/// 命令键。
const KEY_STATUS: u8 = 0x01;
const KEY_EQ: u8 = 0x02;
const KEY_NOISE: u8 = 0x06;
const KEY_DUAL: u8 = 0x08;

/// 读索引（写索引 = 读索引 | `WRITE_FLAG`）。
const IDX_BATTERY: u8 = 0x01;
const IDX_EQ: u8 = 0x01;
const IDX_NOISE_MODE: u8 = 0x02;
const IDX_GAME_MODE: u8 = 0x03;
const IDX_ANC_GRADIENT: u8 = 0x07;
const IDX_DUAL_DEVICE: u8 = 0x02;

/// 降噪分栏的子档：`01` = 固定降噪，`02` = 自适应降噪。
const ANC_FIXED_LEVEL: u8 = 0x01;
const ANC_ADAPTIVE_LEVEL: u8 = 0x02;

/// 电量应答里各字段在 `data` 中的偏移。
const BATTERY_LEFT: usize = 3;
const BATTERY_RIGHT: usize = 4;
const BATTERY_LEFT_CHARGING: usize = 5;
const BATTERY_RIGHT_CHARGING: usize = 6;
const BATTERY_CASE: usize = 7;

/// 写类型：厂商特征同时支持 write 与 write-without-response，取无应答写
/// （与设备「先回显、再回应答帧」的行为一致；写入本身的失败仍由系统上报）。
const WRITE_TYPE: WriteType = WriteType::WithoutResponse;

/// 扫描超时：无过滤扫描是轮询式的，超过这个时间仍未出现就明确报错，不无限重试。
const SCAN_TIMEOUT: Duration = Duration::from_secs(20);
/// 扫描轮询间隔（`peripherals()` 是拉取式，没有「发现事件」可用）。
const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 服务发现超时。
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
/// 单次「写 + 等应答」的应答超时（由会话线程计时，命令层只 await 一次性通道）。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
/// 电量轮询间隔：电量是慢变量，不需要连续读。
const BATTERY_POLL_INTERVAL: Duration = Duration::from_secs(45);
/// 会话心跳：回收超时请求 / 到点轮询电量，顺带让取消能及时被发现。
const TICK: Duration = Duration::from_secs(1);
/// 重连退避的下限与上限（只在**链路建立之后**断开时使用）。
const RECONNECT_MIN_BACKOFF: Duration = Duration::from_secs(2);
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// 候选设备扫描窗口（`scan_ble_devices`，约 8 秒）。
///
/// 耳机的广播间隔很短，8 秒足够扫到；再长只会让「点一下按钮」等太久。
const CANDIDATE_SCAN_WINDOW: Duration = Duration::from_secs(8);
/// 看门狗单轮扫描窗口（约 5 秒）：取出耳机后最多等这么久就能发现它。
const WATCH_SCAN_WINDOW: Duration = Duration::from_secs(5);
/// 看门狗两轮之间的空闲（约 3 秒）：**这是自动连接唯一的重试节奏**，
/// 配合 `WATCH_SCAN_WINDOW` 即「没扫到就约 8 秒后再来一轮」，不会打满适配器。
const WATCH_IDLE: Duration = Duration::from_secs(3);

/// 换会话目标时等待旧会话释放耳机槽位的上限与轮询间隔。
///
/// 旧会话收到停止信号会立刻退出（并 `disconnect`），正常几十毫秒内就能交接完；
/// 上限只是兜底，避免这个后台线程一直等下去。
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(10);
const HANDOVER_POLL: Duration = Duration::from_millis(50);

/// 等待「写回执」的上限（与 `RESPONSE_TIMEOUT` 同一量级）。
///
/// 会话线程在收帧循环里会立刻处理出站消息，正常几毫秒就回执；超过这个时间说明它
/// 根本没在处理消息（正在扫描 / 连接，或已经卡住）—— 那种情况下**宁可报错**，
/// 也不能让命令层返回 `Ok`：前端会以为设置成功而什么都不提示。
/// 超时的语义是「没拿到回执」而不是「一定没写出去」，因此错误文案要说清楚。
const WRITE_ACK_TIMEOUT: Duration = Duration::from_secs(3);

/// 重连等待期间被丢弃的命令回给调用方的错误。
///
/// **绝不能悄悄丢掉**：命令层会返回 `Ok`、前端不弹提示，用户只觉得「点了没反应」。
const RECONNECT_DISCARDED: &str = "耳机正在重连，命令未发送";

/// 当前使用的降噪模式（UI 上的四个分栏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoiseMode {
    /// 降噪（`d1` 另分子档：固定 / 自适应）。
    Anc,
    /// 正常。
    Normal,
    /// 风噪。
    Wind,
    /// 通透。
    Transparency,
}

impl NoiseMode {
    /// 5 字节 `data`：`d0` 恒为 `00`，`d1..d4` 分别对应四个分栏的子档位。
    const fn data(self) -> [u8; 5] {
        match self {
            NoiseMode::Anc => [EMPTY_DATA, ANC_FIXED_LEVEL, 0x00, 0x00, 0x00],
            NoiseMode::Normal => [EMPTY_DATA, 0x00, 0x01, 0x00, 0x00],
            NoiseMode::Wind => [EMPTY_DATA, 0x00, 0x00, 0x01, 0x00],
            NoiseMode::Transparency => [EMPTY_DATA, 0x00, 0x00, 0x00, 0x01],
        }
    }
}

/// 降噪梯度（仅在降噪模式下有意义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AncGradient {
    /// 轻度。
    Light = 0x00,
    /// 中度。
    Medium = 0x01,
    /// 深度。
    Deep = 0x02,
}

impl AncGradient {
    /// 由应答的档位字节反解。
    fn from_level(level: u8) -> Option<Self> {
        match level {
            0x00 => Some(AncGradient::Light),
            0x01 => Some(AncGradient::Medium),
            0x02 => Some(AncGradient::Deep),
            _ => None,
        }
    }
}

/// EQ 音效预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EqPreset {
    /// HIFI。
    Hifi = 0x00,
    /// POP。
    Pop = 0x01,
    /// ROCK。
    Rock = 0x02,
}

impl EqPreset {
    /// 由应答的档位字节反解。
    fn from_level(level: u8) -> Option<Self> {
        match level {
            0x00 => Some(EqPreset::Hifi),
            0x01 => Some(EqPreset::Pop),
            0x02 => Some(EqPreset::Rock),
            _ => None,
        }
    }
}

/// 会话所处的阶段（前端据此渲染连接态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BlePhase {
    /// 未连接（含用户主动断开）。
    Idle,
    /// 正在扫描目标设备。
    Scanning,
    /// 已扫到，正在建立链路。
    Connecting,
    /// 链路就绪，正在收帧。
    Ready,
    /// 失败（`error` 带原因；链路建立失败就此停在 Error，等用户重新连接）。
    Error,
}

/// 一次电量读数。
///
/// `0xFF` 是「不可用」哨兵（充电仓不在位 / 未通信），必须映射为 `None`——
/// 当成 255% 会让 UI 显示一个荒唐的值。充电位 `0` / `1` 之外的取值同样视为未知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryReading {
    /// 左耳电量百分比。
    pub left: Option<u8>,
    /// 右耳电量百分比。
    pub right: Option<u8>,
    /// 充电仓电量百分比。
    pub case: Option<u8>,
    /// 左耳是否在充电。
    pub left_charging: Option<bool>,
    /// 右耳是否在充电。
    pub right_charging: Option<bool>,
    /// 读数时间戳（秒）。
    pub read_at_secs: u64,
}

/// 会话状态快照：命令返回值与 `ble:state` 事件载荷共用同一个结构。
///
/// 字段名是与前端冻结的契约（`camelCase`），不可随意增删或改名。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleStateSnapshot {
    pub phase: BlePhase,
    pub address: Option<String>,
    pub name: Option<String>,
    pub battery: Option<BatteryReading>,
    pub noise_mode: Option<NoiseMode>,
    pub adaptive_anc: Option<bool>,
    pub anc_gradient: Option<AncGradient>,
    pub eq_preset: Option<EqPreset>,
    pub game_mode: Option<bool>,
    pub dual_device: Option<bool>,
    /// 已知设备列表（配置 `ble_devices` 的投影）。
    pub devices: Vec<BleDeviceEntrySerialized>,
    /// 当前 / 上次已连接的目标（配置 `ble_active_address`）。
    pub active_address: Option<String>,
    /// 是否开启自动连接（配置 `ble_auto_connect`）。
    pub auto_connect: bool,
    /// 看门狗是否**正在**轮询目标（见 `Inner::is_watching`）。
    pub watching: bool,
    pub error: Option<String>,
}

/// 快照里的已知设备项：`config::BleDeviceEntry` 的对外投影。
///
/// 与配置同形（`camelCase`），单独一个类型是为了让快照不直接暴露配置模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleDeviceEntrySerialized {
    pub address: String,
    pub name: Option<String>,
    pub last_connected_at: Option<u64>,
}

impl From<&BleDeviceEntry> for BleDeviceEntrySerialized {
    fn from(entry: &BleDeviceEntry) -> Self {
        Self {
            address: entry.address.clone(),
            name: entry.name.clone(),
            last_connected_at: entry.last_connected_at,
        }
    }
}

/// 扫描到的候选设备（`scan_ble_devices` 的返回项）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleCandidate {
    /// 设备地址（`AA:BB:CC:DD:EE:FF`，大写）。
    pub address: String,
    /// 广播名；耳机**不广播名字**，因此通常是缓存名或 `None`。
    pub name: Option<String>,
    /// 广播里的信号强度；平台没报告时为 `0`（不臆造）。
    pub rssi: i16,
    /// 厂商数据原文（小写十六进制）；只取广播里实际存在的字节。
    pub manufacturer_payload: String,
    /// 地址是否已在配置的 `ble_devices` 里。
    pub known: bool,
}

/// 一帧已通过门控 / 长度 / 校验和的应答。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// 命令键。
    pub key: u8,
    /// 命令索引（已抹掉写标志位）。
    pub idx: u8,
    /// 应答数据段。
    pub data: Vec<u8>,
}

/// 和校验：前方所有字节之和 `& 0xFF`。
fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, byte| acc.wrapping_add(*byte))
}

/// 组装一帧请求：`08 EE 00 00 00 <key> <idx> <LEN> <data…> <CKSUM>`。
fn build_request(key: u8, idx: u8, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(DATA_OFFSET + data.len() + 1);
    frame.extend_from_slice(&REQUEST_PREFIX);
    frame.push(key);
    frame.push(idx);
    // LEN 是**整帧总长度**：前缀 8 字节 + data + 校验和 1 字节。
    frame.push((DATA_OFFSET + data.len() + 1) as u8);
    frame.extend_from_slice(data);
    let value = checksum(&frame);
    frame.push(value);
    frame
}

/// 读请求：`data` 恒为单个 `00`。
pub fn read_request(key: u8, idx: u8) -> Vec<u8> {
    build_request(key, idx, &[EMPTY_DATA])
}

/// 写请求：`idx` 最高位置 1（读 / 写共用同一组 `key`/`idx`）。
pub fn write_request(key: u8, read_idx: u8, data: &[u8]) -> Vec<u8> {
    build_request(key, read_idx | WRITE_FLAG, data)
}

/// 电量读请求。
pub fn battery_request() -> Vec<u8> {
    read_request(KEY_STATUS, IDX_BATTERY)
}

/// 噪声模式（降噪 / 正常 / 风噪 / 通透）写帧。
pub fn noise_mode_request(mode: NoiseMode) -> Vec<u8> {
    write_request(KEY_NOISE, IDX_NOISE_MODE, &mode.data())
}

/// 自适应降噪开关：`d1 = 2` 开、`d1 = 1` 关。
///
/// **这一帧是对的**：用 patch 过的厂商 App 抓 logcat，用户点这个开关时厂商发的
/// 就是**和我们一模一样**的两帧（逐字节相同，时间戳 `11:49:48.320` / `11:49:52.679`）：
/// ```text
/// TX  08 EE 00 00 00 06 82 0E 00 02 00 00 00 8E   ← 自适应降噪 开
/// TX  08 EE 00 00 00 06 82 0E 00 01 00 00 00 8D   ← 自适应降噪 关
/// ```
/// 所以 `00 02 00 00 00` **不是**一个非法模式值，不要把它当成错的帧去改。
///
/// 真正的情况是**设备不回话、也读不回来**：
///
/// 1. 厂商 App 发出 `00 02` 之后，设备**什么都没有回** —— 没有 ACK，也没有推送，
///    日志下一行就是 4 秒后用户的下一个动作；
/// 2. A/B 实测（官方 App 里让自适应降噪保持开 vs 关），把厂商 App 自己读的**全部 14 个
///    字段**都读了一遍（`01/01`、`02/01`、`06/02`、`06/03`、`08/02`、`0c/01`、`08/01`、
///    `0e/03`、`04/01`、`0a/01`、`06/07`、`06/04`、`06/05`、`0d/01`）—— **逐字节完全相同**，
///    也就是说这个状态**在可读字段里根本不存在**（`06/02` 永远只报固定/自适应里的
///    固定那一档，见 `decode_noise`），厂商 App 只能自己在本地记住它。
///
/// 结论（**已定：本地记忆**）：既然设备不上报，UI 就不能靠「读回设备值」来驱动 ——
/// 我们补的读回（见 `write_readback`）会让开关**立刻弹回关**，看起来像坏了。
/// 因此这个开关改为**我们自己记住**：值存在配置的设备条目里
/// （`BleDeviceEntry::adaptive_anc`，见 `remember_adaptive_anc`），快照的 `adaptiveAnc`
/// 一律从那里取（`Inner::remembered_adaptive_anc`），设备报文对它没有发言权
/// （`Inner::apply` 明确不写这一位）。厂商 App 显然也是这么做的。帧本身照发不误 ——
/// 它和厂商发的逐字节相同，设备收到后是否真的生效我们无从验证，但发是必须的。
///
/// 另外：`06/04` 也不是自适应降噪，它是「通透」那一栏的可写子档（写它会改掉 `06/04`，
/// 并让设备把「通透」推成当前模式）—— 这条结论不变。
pub fn adaptive_anc_request(enabled: bool) -> Vec<u8> {
    let level = if enabled {
        ANC_ADAPTIVE_LEVEL
    } else {
        ANC_FIXED_LEVEL
    };
    let data = [EMPTY_DATA, level, 0x00, 0x00, 0x00];
    write_request(KEY_NOISE, IDX_NOISE_MODE, &data)
}

/// 游戏模式写帧：`0` = 开，`1` = 关。
pub fn game_mode_request(enabled: bool) -> Vec<u8> {
    let level = if enabled { 0x00 } else { 0x01 };
    write_request(KEY_NOISE, IDX_GAME_MODE, &[EMPTY_DATA, level])
}

/// 降噪梯度写帧：`0` = 轻度、`1` = 中度、`2` = 深度。
pub fn anc_gradient_request(level: AncGradient) -> Vec<u8> {
    write_request(KEY_NOISE, IDX_ANC_GRADIENT, &[EMPTY_DATA, level as u8])
}

/// 双设备连接写帧：`0` = 关，`1` = 开。
pub fn dual_device_request(enabled: bool) -> Vec<u8> {
    write_request(KEY_DUAL, IDX_DUAL_DEVICE, &[EMPTY_DATA, u8::from(enabled)])
}

/// EQ 音效写帧：`0` = HIFI、`1` = POP、`2` = ROCK。
pub fn eq_preset_request(preset: EqPreset) -> Vec<u8> {
    write_request(KEY_EQ, IDX_EQ, &[EMPTY_DATA, preset as u8])
}

/// 写降噪梯度时**必须随后重发一次当前模式**。
///
/// 实测：写完梯度，耳机会隐含把自己的噪声模式切回「降噪」——用户选的模式会被静默覆盖。
/// 因此本函数返回「梯度帧 + 模式帧」这一对；模式未知时退化为读回一次模式，
/// 让界面至少能与设备的真实状态重新对齐（而不是继续显示一个已经被改掉的旧值）。
///
/// 实测（补充复核）：梯度写本身是**有效**的 —— 依次写 `06/07` = 0 / 1 / 2 再读回，
/// 分别得到 0 / 1 / 2，每次都真的变了；并且**写某一栏的子档会隐含把那一栏选为当前模式**
/// （写梯度后设备立刻推送 `06/02 = 00 01 00 00 00`，即降噪）。所以梯度唯一的毛病只是
/// 「写应答不带新值」，由 `write_readback` 补读解决；这里保留「梯度帧 + 模式帧」的顺序，
/// 是把被隐含改掉的模式恢复回去。
///
/// 会话线程会给每个写帧再补一帧同 `key`/`idx` 的读（见 `write_readback`），
/// 因此这一对帧实际发出去是「梯度写 → 梯度读 → 模式写 → 模式读」：
/// 先确认梯度，再把设备隐含改掉的模式恢复并确认。
pub fn anc_gradient_frames(level: AncGradient, current: Option<NoiseMode>) -> Vec<Vec<u8>> {
    let restore = match current {
        Some(mode) => noise_mode_request(mode),
        None => read_request(KEY_NOISE, IDX_NOISE_MODE),
    };
    vec![anc_gradient_request(level), restore]
}

/// 一个**写帧**应当补上的「读回」帧；读帧返回 `None`。
///
/// 设备对写请求只回一个**单字节 ACK**（`09 FF 00 00 01 <key> <idx|0x80> 0A 00 <CK>`，
/// 即 `data = 00`），而 `Inner::apply` 对 EQ / 降噪梯度 / 游戏模式 / 双设备这几项需要
/// `data[1]`（≥ 2 字节）才能解出新值 —— 写成功之后快照于是长期停在旧值，直到别的原因
/// 触发一次读（用户看到的「点完不动，重开小窗才对」，就是重开小窗触发了读）。
/// 实测只有噪声模式会被设备**主动推送**，其余项必须自己读回来。所以这里不按命令手工
/// 配对，而是直接从帧本身推导：`idx` 的最高位（`WRITE_FLAG`）就是「这是写」的标志。
///
/// 顺带一提：**1 字节的 `00` 应答不携带任何状态**（它既是写 ACK，也是「读了一个不支持的
/// `idx`」的回法），因此 `Inner::apply` 对每个键都要求 ≥ 2 字节，取不到就什么都不改。
///
/// **背靠背发送是安全的（实测，不是假设）**：写帧与读帧之间不加任何延时，设备回复的
/// 顺序固定为「写回声 → 写 ACK → 读回声 → 读应答」，且读应答里已经是**新值**
/// （EQ `00 00` → 写 `00 01` → 读回 `00 01`；游戏模式 `00 01` → 写 `00 00` → 读回 `00 00`），
/// 即设备先落写、再处理读。**不要为了「稳妥」加延时或重试**，那只会拖慢每次设置。
fn write_readback(frame: &[u8]) -> Option<Vec<u8>> {
    // 读帧（初始刷新、电量轮询）没有写标志位，因此不会在读后面再补一次读。
    let idx = *frame.get(IDX_OFFSET)?;
    if idx & WRITE_FLAG == 0 {
        return None;
    }
    let key = *frame.get(KEY_OFFSET)?;
    Some(read_request(key, idx & IDX_MASK))
}

/// 这一帧是不是**我们自己的请求**被设备原样回显回来的。
///
/// 回显帧会被 `parse_response` 挡掉，而这是**设计如此**：回显帧同样满足校验和规则
/// （`09 FF` 门控正是为它而存在），放进来会被当成合法应答，且它的 `idx` 还带着写标志位。
/// 但它属于预期内的正常现象（设备把每个请求都回显一遍），不能与真正无法解析的帧
/// 混在一起刷 debug 日志 —— 否则用户只会看到「一堆无法解析蓝牙通知」。
fn is_request_echo(frame: &[u8]) -> bool {
    frame.starts_with(&REQUEST_PREFIX)
}

/// 帧的可读摘要（日志专用）：`key=0x06 idx=0x07 data=00 01`。
///
/// **原样给字节**：`idx` 不抹写标志位（设备的回显 / ACK 本身就带标志位），data 也不解码 ——
/// 这样「我们要求什么」与「设备报回来什么」两行可以并排人肉对照。太短的帧返回 `None`
/// （出站帧与已解析的应答都不会这样，纯粹是防御）。
fn frame_summary(frame: &[u8]) -> Option<String> {
    let key = *frame.get(KEY_OFFSET)?;
    let idx = *frame.get(IDX_OFFSET)?;
    let data = frame.get(DATA_OFFSET..frame.len().checked_sub(1)?)?;
    Some(format!("key={key:#04X} idx={idx:#04X} data={}", hex(data)))
}

/// 打一条 DEBUG 日志：**我们发出去的设置写帧**（读帧不打，免得刷屏）。
///
/// 与 `handle_notification` 里那条「设备报回来什么」配对使用：
/// ```text
/// 写入设置 key=0x06 idx=0x87 data=00 01
/// 收到应答 key=0x06 idx=0x87 data=00 00     ← 设备没接受（或没带上新值）
/// ```
/// 反过来，如果是我们自己的读回机制坏了，日志里就**只有写入、没有应答**。
///
/// **不在日志里下结论**（不写「一致 / 不一致」）：判断需要跨行的在途状态，写错一次就会
/// 把读日志的人带偏 —— 两行并排看，结论由人来做。
fn log_setting_write(frame: &[u8]) {
    let Some(idx) = frame.get(IDX_OFFSET) else {
        return;
    };
    if idx & WRITE_FLAG == 0 {
        return;
    }
    if let Some(summary) = frame_summary(frame) {
        log::debug!("写入设置 {summary}");
    }
}

/// 读一遍全部设置项（连接就绪时与用户手动刷新时用）。
pub fn refresh_requests() -> Vec<Vec<u8>> {
    vec![
        battery_request(),
        read_request(KEY_EQ, IDX_EQ),
        read_request(KEY_NOISE, IDX_NOISE_MODE),
        read_request(KEY_NOISE, IDX_GAME_MODE),
        read_request(KEY_NOISE, IDX_ANC_GRADIENT),
        read_request(KEY_DUAL, IDX_DUAL_DEVICE),
    ]
}

/// 解析一帧通知：**回声门控 → 长度校验 → 校验和校验**。
///
/// 顺序很关键：耳机会把请求原样回显到 notify 特征，而**回显帧同样满足校验和规则**，
/// 所以必须先用 `09 FF` 把头挡掉，否则回声会被当成合法应答（`08 EE` 帧的 `idx`
/// 还带写标志位，一旦漏进来就会污染状态）。
pub fn parse_response(frame: &[u8]) -> Option<Response> {
    if !frame.starts_with(&RESPONSE_PREFIX) {
        return None;
    }
    // LEN 是整帧总长度：既不能与真实长度不符，也不能短到装不下 `data` + 校验和。
    let length = usize::from(*frame.get(LEN_OFFSET)?);
    if length != frame.len() || length <= DATA_OFFSET {
        return None;
    }
    let (body, trailing) = frame.split_at(frame.len() - 1);
    if checksum(body) != trailing[0] {
        return None;
    }
    Some(Response {
        key: frame[KEY_OFFSET],
        idx: frame[IDX_OFFSET] & IDX_MASK,
        data: frame[DATA_OFFSET..frame.len() - 1].to_vec(),
    })
}

/// 解码电量应答的 `data`；长度不足时返回 `None`。
pub fn decode_battery(data: &[u8], read_at_secs: u64) -> Option<BatteryReading> {
    Some(BatteryReading {
        left: percent(*data.get(BATTERY_LEFT)?),
        right: percent(*data.get(BATTERY_RIGHT)?),
        case: percent(*data.get(BATTERY_CASE)?),
        left_charging: charging(*data.get(BATTERY_LEFT_CHARGING)?),
        right_charging: charging(*data.get(BATTERY_RIGHT_CHARGING)?),
        read_at_secs,
    })
}

/// 电量字节 → 百分比：`0xFF` 是「不可用」哨兵，不是 255%。
const fn percent(raw: u8) -> Option<u8> {
    if raw == UNAVAILABLE {
        None
    } else {
        Some(raw)
    }
}

/// 充电标志字节 → 布尔：`1` 充电、`0` 未充电、其它取值未知。
const fn charging(raw: u8) -> Option<bool> {
    match raw {
        0x00 => Some(false),
        0x01 => Some(true),
        _ => None,
    }
}

/// 由应答的 5 字节 `data` 反解噪声模式与「降噪栏的子档是不是自适应」。
///
/// **第二个返回值不再用来填快照**（`Inner::apply` 明确丢掉它，理由见那里的注释）：
/// 实测（A/B 覆盖厂商 App 自己读的全部 14 个字段，详见 `adaptive_anc_request`）
/// `06/02` **从不报告**自适应那一档 —— 无论自适应降噪是开是关，读回来的 `d1` 都是 `01`
/// （固定降噪）。而写请求后面都会补一次读回，所以用它填 `adaptiveAnc` 会让开关每次都被
/// 打回 false。保留这个返回值只是因为它是这段字节模式的**正确解码**（纯函数，单测仍然成立），
/// 也留作将来固件真的开始上报时的入口；快照里的 `adaptiveAnc` 一律取自本地记忆
/// （`Inner::remembered_adaptive_anc`）。
fn decode_noise(data: &[u8]) -> Option<(NoiseMode, bool)> {
    let payload = data.get(..5)?;
    // `d0` 实测恒为 00，只是容器前缀，不参与判定。
    if payload[1] != 0x00 {
        return Some((NoiseMode::Anc, payload[1] == ANC_ADAPTIVE_LEVEL));
    }
    if payload[2] != 0x00 {
        return Some((NoiseMode::Normal, false));
    }
    if payload[3] != 0x00 {
        return Some((NoiseMode::Wind, false));
    }
    if payload[4] != 0x00 {
        return Some((NoiseMode::Transparency, false));
    }
    None
}

/// 落值并报告是否真的变了（只有变化才需要广播）。
fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// 会话内部状态（`BleStateSnapshot` 是它的对外投影）。
///
/// 分两类字段：
/// - **会话字段**（`phase` … `error`）：属于「这一轮连接」，`reset_session` 一律清空；
/// - **配置镜像 / 看门狗字段**（`devices` … `watch_generation`）：跨会话保留，
///   只由 `sync_from_config` 与看门狗自己改。
#[derive(Debug, Clone, PartialEq)]
struct Inner {
    phase: BlePhase,
    address: Option<String>,
    name: Option<String>,
    battery: Option<BatteryReading>,
    noise_mode: Option<NoiseMode>,
    anc_gradient: Option<AncGradient>,
    eq_preset: Option<EqPreset>,
    game_mode: Option<bool>,
    dual_device: Option<bool>,
    error: Option<String>,
    /// 已知设备列表（`AppConfig::ble_devices` 的镜像）。
    devices: Vec<BleDeviceEntry>,
    /// 自动连接的目标（`AppConfig::ble_active_address` 的镜像）。
    active_address: Option<String>,
    /// 是否开启自动连接（`AppConfig::ble_auto_connect` 的镜像）。
    auto_connect: bool,
    /// 本轮会话要连的地址（会话开始时定下）：`connect_earbuds` 靠它判断
    /// 「已有会话是不是就在连我要的那台」，从而对重复请求保持幂等。
    session_target: Option<String>,
    /// 看门狗线程是否在跑（快照里的 `watching` 还要看阶段与抑制标志）。
    watcher_running: bool,
    /// 用户主动断开后的抑制标志：抑制期间看门狗**不自动重连**（否则用户永远断不开）。
    suppressed: bool,
    /// 看门狗轮次代号：`stop_watch` / `begin_watch` 各自自增，
    /// 在途的一轮发现代号不等于自己即退出（与 `generation` 同一套作废手法）。
    watch_generation: u64,
}

impl Inner {
    fn idle() -> Self {
        Self {
            phase: BlePhase::Idle,
            address: None,
            name: None,
            battery: None,
            noise_mode: None,
            anc_gradient: None,
            eq_preset: None,
            game_mode: None,
            dual_device: None,
            error: None,
            devices: Vec::new(),
            active_address: None,
            auto_connect: false,
            session_target: None,
            watcher_running: false,
            suppressed: false,
            watch_generation: 0,
        }
    }

    /// 复位**会话字段**（配置镜像与看门狗字段跨会话保留）。
    ///
    /// 断开连接不该把已知设备列表 / 自动连接开关从快照里抹掉：前端还要靠它们渲染列表，
    /// 看门狗也要靠 `active_address` 决定盯谁。
    fn reset_session(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        let active_address = self.active_address.take();
        let auto_connect = self.auto_connect;
        let watcher_running = self.watcher_running;
        let suppressed = self.suppressed;
        let watch_generation = self.watch_generation;
        *self = Inner {
            devices,
            active_address,
            auto_connect,
            watcher_running,
            suppressed,
            watch_generation,
            ..Inner::idle()
        };
    }

    /// 快照里的 `watching`：看门狗真的在轮询时才是 `true`。
    ///
    /// 三个条件缺一不可：看门狗在跑、自动连接开着且有目标、**当前没有会话在跑**
    /// （`scanning` / `connecting` / `ready` 都算有会话；`idle` / `error` 没有，
    /// 因为会话线程在这两个阶段已经结束或即将结束）。抑制标志也一并参与判定。
    fn is_watching(&self) -> bool {
        self.watcher_running
            && self.auto_connect
            && self.active_address.is_some()
            && !self.suppressed
            && matches!(self.phase, BlePhase::Idle | BlePhase::Error)
    }

    /// 「当前设备」的地址：**连接中的设备优先**，其次是当前 / 上次连接的目标。
    ///
    /// 本地记忆类偏好（自适应降噪）读写都用这一个口径：用户操作的是「现在这台」，
    /// 还没连上时就是「正要去连的那台」。
    fn current_device(&self) -> Option<&str> {
        self.address.as_deref().or(self.active_address.as_deref())
    }

    /// 指定设备的**本地记忆**自适应降噪开关；没有条目时为 `false`（等于出厂默认）。
    fn remembered_adaptive_anc_of(&self, address: &str) -> bool {
        self.devices
            .iter()
            .find(|entry| entry.address.eq_ignore_ascii_case(address))
            .is_some_and(|entry| entry.adaptive_anc)
    }

    /// 快照里的 `adaptiveAnc`：**本地记忆**，不是设备报的。
    ///
    /// 设备从不上报这一位 —— `06/02` 永远只报「固定降噪」，A/B 覆盖厂商 App 自己读的
    /// 全部 14 个字段也全都相同（详见 `adaptive_anc_request`）。所以这里只能从配置镜像
    /// （`devices`）里取当前设备记住的值；连目标都没有（什么都没连过）时为 `None`（未知）。
    ///
    /// 每次派生（而不是在「会话建立 / 换设备」时拷进某个字段）是为了**从结构上**保证
    /// 换耳机时显示的是那台耳机自己的记忆值：`establish` 填 `address`、
    /// `apply_preferences` / `remember_connection` 改 `active_address` 或 `devices` 之后，
    /// 下一个快照自然就是对的，不存在「忘了同步」这一类状态。
    fn remembered_adaptive_anc(&self) -> Option<bool> {
        Some(self.remembered_adaptive_anc_of(self.current_device()?))
    }

    fn set_phase(&mut self, phase: BlePhase) -> bool {
        replace(&mut self.phase, phase)
    }

    fn set_error(&mut self, error: Option<String>) -> bool {
        replace(&mut self.error, error)
    }

    fn set_connection(&mut self, address: Option<String>, name: Option<String>) -> bool {
        let address_changed = replace(&mut self.address, address);
        let name_changed = replace(&mut self.name, name);
        address_changed || name_changed
    }

    /// 只更新设备名（读回 GATT Device Name 之后调用）。
    fn set_name(&mut self, name: Option<String>) -> bool {
        replace(&mut self.name, name)
    }

    /// 把一帧应答并入状态，返回是否有字段真的变了。
    ///
    /// 每条分支都要求 **≥ 2 字节** `data`：实测 1 字节的 `00` 应答**不携带任何状态**
    /// （它既是写操作的 ACK，也是「读了一个不支持的 `idx`」的回法），把 `00` 当状态读进去
    /// 会写坏快照。因此写请求的新值只能靠会话线程补的那一次读回来（见 `write_readback`）。
    fn apply(&mut self, response: &Response) -> bool {
        match (response.key, response.idx) {
            (KEY_STATUS, IDX_BATTERY) => match decode_battery(&response.data, now_secs()) {
                Some(reading) => replace(&mut self.battery, Some(reading)),
                None => false,
            },
            (KEY_EQ, IDX_EQ) => {
                match response.data.get(1).copied().and_then(EqPreset::from_level) {
                    Some(preset) => replace(&mut self.eq_preset, Some(preset)),
                    None => false,
                }
            }
            (KEY_NOISE, IDX_NOISE_MODE) => match decode_noise(&response.data) {
                // **只并入噪声模式，绝不碰自适应降噪**（这条看着像漏了，其实是有意的）：
                // 设备在 `06/02` 里永远只报「固定降噪」那一档，从不上报自适应位
                // （见 `decode_noise` 与 `adaptive_anc_request` 的实测记录）。而写请求后面
                // 都会补一次读回（`write_readback`），所以这里一旦把 `d1 = 01` 解出来的
                // `false` 写进快照，用户每点一次开关就会被读回**打回 false** ——
                // 「开关弹回来、像坏了」正是这个原因。因此 `adaptiveAnc` 只从本地记忆
                // （`Inner::remembered_adaptive_anc`）取，设备报文对它没有发言权。
                Some((mode, _)) => replace(&mut self.noise_mode, Some(mode)),
                None => false,
            },
            (KEY_NOISE, IDX_GAME_MODE) => match response.data.get(1).copied() {
                // 游戏模式：0 = 开、1 = 关。
                Some(level) => replace(&mut self.game_mode, Some(level == 0x00)),
                None => false,
            },
            (KEY_NOISE, IDX_ANC_GRADIENT) => {
                match response
                    .data
                    .get(1)
                    .copied()
                    .and_then(AncGradient::from_level)
                {
                    Some(level) => replace(&mut self.anc_gradient, Some(level)),
                    None => false,
                }
            }
            (KEY_DUAL, IDX_DUAL_DEVICE) => match response.data.get(1).copied() {
                // 双设备连接：0 = 关、1 = 开。
                Some(level) => replace(&mut self.dual_device, Some(level != 0x00)),
                None => false,
            },
            _ => false,
        }
    }
}

/// 命令层发往会话线程的出站消息。
enum Outgoing {
    /// 写一帧。
    ///
    /// `reply` **只回报「这一帧有没有真的写出去」**，不回报设备应答（状态由 `ble:state`
    /// 推送）。它存在的唯一理由是：帧可能因为链路刚断、正在退避重连而被丢弃，而
    /// 命令层若在这种情况下返回 `Ok`，前端就不会弹任何提示 —— 用户看到的是
    /// 「点了没反应，也没有报错」（真机日志里 `链路重连中，忽略一条蓝牙命令` 就是它）。
    Write {
        frame: Vec<u8>,
        reply: oneshot::Sender<AppResult<()>>,
    },
    /// 写一帧并等待同一 `key`/`idx` 的应答。
    Request {
        key: u8,
        idx: u8,
        frame: Vec<u8>,
        reply: oneshot::Sender<AppResult<Response>>,
    },
    /// 请求会话线程退出（用户主动断开）。
    Stop,
}

/// 一个已投出、正在等应答的请求。
struct Pending {
    key: u8,
    idx: u8,
    deadline: tokio::time::Instant,
    reply: oneshot::Sender<AppResult<Response>>,
}

/// btleplug 的通知流类型。
type Notifications = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;

/// 一条已建立并订阅好的链路。
struct Established {
    peripheral: Peripheral,
    write: Characteristic,
    notifications: Notifications,
}

/// 一轮会话的结束方式。
enum RoundOutcome {
    /// 被取消 / 出站通道关闭：整轮会话结束，不再重连。
    Stopped,
    /// 链路建立之后断开：退避后重连。
    Dropped,
}

/// 由 Tauri 托管的蓝牙会话状态。
pub struct BleState {
    inner: RwLock<Inner>,
    /// 每次会话自增；在途会话发现自己的代号过期即放弃后续改动。
    generation: AtomicU64,
    cancel: AtomicBool,
    /// 出站通道：`Some` 表示会话线程在跑，`None` 表示未连接。
    outbox: Mutex<Option<UnboundedSender<Outgoing>>>,
}

impl BleState {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::idle()),
            generation: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            outbox: Mutex::new(None),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn outbox(&self) -> std::sync::MutexGuard<'_, Option<UnboundedSender<Outgoing>>> {
        self.outbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 当前状态快照。
    pub fn snapshot(&self) -> BleStateSnapshot {
        dto(&self.read())
    }

    /// 主动广播一次当前状态（挂载 / 用户操作等由外部触发的时机）。
    pub fn broadcast(&self, app: &AppHandle) {
        emit_state(app, &self.snapshot());
    }

    /// 本轮会话是否已作废（被取消，或已有更新的一轮开始）。
    fn is_stale(&self, generation: u64) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.generation.load(Ordering::SeqCst) != generation
    }

    /// 是否有会话线程在跑（`connecting` / `ready` / 退避重连中）。
    ///
    /// 看门狗据此保证**连接期间绝不扫描**（耳机一次只接受一个 LE 中央设备，
    /// 扫描会和链路争用适配器）。出站通道是「会话是否存在」的准确依据。
    fn is_session_active(&self) -> bool {
        self.outbox().is_some()
    }

    /// 是否处于「用户主动断开」的抑制状态。
    fn is_suppressed(&self) -> bool {
        self.read().suppressed
    }

    /// 本轮会话要连的地址（没有会话时为 `None`）。
    fn session_target(&self) -> Option<String> {
        self.read().session_target.clone()
    }

    /// 已知设备列表（配置镜像的克隆）：`scan_ble_devices` 用它标记 `known`。
    pub fn known_devices(&self) -> Vec<BleDeviceEntry> {
        self.read().devices.clone()
    }

    /// 这台地址是不是当前会话正在连接 / 已连接的设备。
    ///
    /// 两个都看：`session_target` 是「正要去连的」，`address` 是「已经连上的」。
    pub fn owns_address(&self, address: &str) -> bool {
        let inner = self.read();
        let owned = [inner.session_target.as_deref(), inner.address.as_deref()]
            .into_iter()
            .flatten()
            .any(|current| current.eq_ignore_ascii_case(address));
        owned
    }

    /// 以会话代号为前提改状态。
    ///
    /// 代号过期时返回 `None`（调用方据此停止），否则返回新快照与「是否有字段真的变了」
    /// ——只有变化才需要广播，避免事件风暴（照 `file_index.rs` 的 `is_stale` 门控）。
    fn commit<F>(&self, generation: u64, mutate: F) -> Option<(BleStateSnapshot, bool)>
    where
        F: FnOnce(&mut Inner) -> bool,
    {
        if self.is_stale(generation) {
            return None;
        }
        let mut inner = self.write();
        let changed = mutate(&mut inner);
        Some((dto(&inner), changed))
    }

    /// 开始一轮会话：已在会话中时返回 `None`（幂等）；否则作废上一轮并装好出站通道。
    ///
    /// `target` 是本轮要连的地址（只用于「连谁」的判定；真正的扫描目标每轮都由
    /// `connect_target` 从配置重新解析）。
    fn begin_session(&self, target: Option<String>) -> Option<(u64, UnboundedReceiver<Outgoing>)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        {
            let mut outbox = self.outbox();
            if outbox.is_some() {
                // 已在会话中：刚建好的通道随 `sender` 一起丢弃。
                return None;
            }
            *outbox = Some(sender);
        }

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel.store(false, Ordering::SeqCst);
        let mut inner = self.write();
        inner.reset_session();
        inner.session_target = target;
        drop(inner);
        Some((generation, receiver))
    }

    /// 会话线程收尾：清掉出站通道，此后命令层会明确报「未连接」。
    ///
    /// 以代号为前提：已有新一轮会话时什么都不做，避免把新会话的通道清掉。
    fn end_session(&self, generation: u64) {
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        *self.outbox() = None;
    }

    /// 用户主动断开：置取消标志、通知会话线程退出，并立刻落回 `idle`。
    ///
    /// 只置 `cancel`（**不动代号**）：会话线程仍要能认出自己并完成收尾。
    fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        // 用户明确要求断开：残留的旧读数会误导界面，全部清掉
        // （配置镜像与看门狗字段由 `reset_session` 保留）。
        self.write().reset_session();
        let outbox = self.outbox();
        if let Some(sender) = outbox.as_ref() {
            // 会话线程收到 Stop 会立即退出，不必等下一个心跳。
            let _ = sender.send(Outgoing::Stop);
        }
    }

    /// 把配置里的三个 BLE 键灌进快照镜像，返回是否有变化（只有变化才需要广播）。
    fn apply_preferences(&self, config: &AppConfig) -> bool {
        let mut inner = self.write();
        let devices_changed = replace(&mut inner.devices, config.ble_devices.clone());
        let address_changed = replace(&mut inner.active_address, config.ble_active_address.clone());
        let auto_changed = replace(&mut inner.auto_connect, config.ble_auto_connect);
        devices_changed || address_changed || auto_changed
    }

    /// 置「用户主动断开」的抑制标志（看门狗据此不自动重连）。返回是否有变化。
    fn suppress(&self) -> bool {
        let mut inner = self.write();
        replace(&mut inner.suppressed, true)
    }

    /// 清掉抑制标志（用户显式连接 / 切换自动连接开关）。返回是否有变化。
    fn clear_suppressed(&self) -> bool {
        let mut inner = self.write();
        replace(&mut inner.suppressed, false)
    }

    /// 现在该不该有看门狗：自动连接开着、有已知目标、且没被用户抑制。
    fn watch_wanted(&self) -> bool {
        let inner = self.read();
        inner.auto_connect && inner.active_address.is_some() && !inner.suppressed
    }

    /// 启动看门狗轮次：已在跑时是空操作（返回 `None`），否则返回本轮代号。
    fn begin_watch(&self) -> Option<u64> {
        let mut inner = self.write();
        if inner.watcher_running {
            return None;
        }
        inner.watcher_running = true;
        inner.watch_generation += 1;
        Some(inner.watch_generation)
    }

    /// 停止看门狗（幂等）：作废在途的一轮，在途线程发现代号过期即退出。
    ///
    /// 即使当前没在跑也自增代号：这样**上一轮迟到的线程**同样会被作废。
    fn stop_watch(&self) -> bool {
        let mut inner = self.write();
        inner.watch_generation += 1;
        replace(&mut inner.watcher_running, false)
    }

    /// 本轮看门狗是否仍是当前轮次（否则说明已被停止 / 被更新的一轮取代）。
    fn watch_is_current(&self, generation: u64) -> bool {
        self.read().watch_generation == generation
    }

    /// 看门狗线程自行收尾：只在仍是当前轮次时清掉「在跑」标记。
    fn end_watch(&self, generation: u64) -> bool {
        let mut inner = self.write();
        if inner.watch_generation != generation {
            return false;
        }
        replace(&mut inner.watcher_running, false)
    }

    /// 投一条出站消息。未连接时返回明确错误（而不是静默丢弃）。
    fn enqueue(&self, outgoing: Outgoing) -> AppResult<()> {
        let outbox = self.outbox();
        let Some(sender) = outbox.as_ref() else {
            return Err(AppError::Other("蓝牙未连接（请先连接耳机）".into()));
        };
        sender
            .send(outgoing)
            .map_err(|_| AppError::Other("蓝牙会话已结束（请重新连接）".into()))
    }
}

impl Default for BleState {
    fn default() -> Self {
        Self::new()
    }
}

fn dto(inner: &Inner) -> BleStateSnapshot {
    BleStateSnapshot {
        phase: inner.phase,
        address: inner.address.clone(),
        name: inner.name.clone(),
        battery: inner.battery,
        noise_mode: inner.noise_mode,
        anc_gradient: inner.anc_gradient,
        eq_preset: inner.eq_preset,
        game_mode: inner.game_mode,
        dual_device: inner.dual_device,
        devices: inner
            .devices
            .iter()
            .map(BleDeviceEntrySerialized::from)
            .collect(),
        active_address: inner.active_address.clone(),
        auto_connect: inner.auto_connect,
        // `adaptiveAnc` 是**本地记忆**，不是设备值：见 `Inner::remembered_adaptive_anc`。
        adaptive_anc: inner.remembered_adaptive_anc(),
        watching: inner.is_watching(),
        error: inner.error.clone(),
    }
}

fn emit_state(app: &AppHandle, state: &BleStateSnapshot) {
    // 广播失败不该影响会话本身：前端还有 get_ble_state 兜底。
    if let Err(err) = app.emit(BLE_STATE_EVENT, state) {
        log::warn!("广播蓝牙状态失败：{err}");
    }
}

/// 以会话代号为前提改状态，并在有变化时广播。返回 `false` 表示本轮会话已过期。
fn commit<F>(app: &AppHandle, generation: u64, mutate: F) -> bool
where
    F: FnOnce(&mut Inner) -> bool,
{
    let state = app.state::<BleState>();
    let Some((next, changed)) = state.commit(generation, mutate) else {
        return false;
    };
    if changed {
        emit_state(app, &next);
    }
    true
}

fn is_stale(app: &AppHandle, generation: u64) -> bool {
    app.state::<BleState>().is_stale(generation)
}

/// 启动会话线程（扫描 → 连接 → 订阅 → 收帧）。已在会话中时是空操作。
pub fn start_session(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<BleState>();
    // 目标地址先解析一次并记进状态：`connect_earbuds` 靠它识别重复请求。
    // 这里不报解析错误（`establish` 会带着完整原因报出来），记 `None` 即可。
    let target = connect_target(app).ok().map(|target| target.to_string());
    let Some((generation, outbox)) = state.begin_session(target) else {
        return Ok(());
    };

    let app = app.clone();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                log::error!("创建蓝牙会话运行时失败：{err}");
                commit(&app, generation, |inner| {
                    let changed = inner.set_phase(BlePhase::Error);
                    inner.set_error(Some(format!("创建蓝牙会话运行时失败：{err}"))) || changed
                });
                app.state::<BleState>().end_session(generation);
                return;
            }
        };
        runtime.block_on(session_loop(&app, generation, outbox));
    });
    Ok(())
}

/// 结束当前会话（幂等）。会话线程会自行收尾，状态立刻落回 `idle`。
///
/// **不抑制看门狗**：这是配置换目标等内部路径用的出口，自动连接应当继续工作。
pub fn stop_session(app: &AppHandle) {
    let state = app.state::<BleState>();
    state.request_cancel();
    state.broadcast(app);
}

/// 用户主动断开（`disconnect_earbuds` 的实现）。
///
/// 除了结束会话，还要置**抑制标志**并停掉看门狗：否则看门狗会在几秒内把链路重新
/// 建起来，用户就永远断不开。抑制由 `connect_earbuds` 或切换自动连接开关清除。
pub fn disconnect_earbuds(app: &AppHandle) {
    let state = app.state::<BleState>();
    state.suppress();
    state.request_cancel();
    state.stop_watch();
    state.broadcast(app);
}

/// 用户显式连接（`connect_earbuds` 的实现）。
///
/// **幂等**：已经在连 / 已连上同一台设备时什么都不做，绝不拆掉重来。
/// 只有「请求的目标与当前会话的目标不同」才会停旧起新（见 `switch_session_target`）。
pub fn connect_earbuds(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<BleState>();
    // 用户明确要连：清掉「不再自动连接」的抑制标志，并确保看门狗在跑（幂等）。
    if state.clear_suppressed() {
        state.broadcast(app);
    }
    start_watch(app);

    let target = connect_target(app)?.to_string();
    if state.is_session_active() {
        // 前端的行内「连接 / 切换」按钮是「先 `set_ble_active_device` 再 `connect`」，
        // 而换目标本身（`sync_from_config`）可能已经起了会话 —— 那这一次 connect 是
        // **重复请求**，重来一遍只会白占一次耳机唯一的 LE 中央槽位、并闪断刚建好的链路。
        if state.session_target().as_deref() == Some(target.as_str()) {
            return Ok(());
        }
        // 目标确实不同：旧会话必须先收尾（它退出时会 `disconnect` 释放耳机唯一的槽位），
        // 因此只发起「停旧 + 等槽位释放后起新」，命令本身立刻返回。
        switch_session_target(app, target);
        return Ok(());
    }
    start_session(app)
}

/// 蓝牙配置的**唯一同步入口**：把配置灌进状态镜像，并让运行时跟上
/// （起停看门狗、换会话目标）。调用方有两类，共用这一条路径：
///
/// - `set_config`（整份配置回写，但三个 BLE 键已被 `preserve_config_keys` 换回权威值，
///   因此那里恒为无变化；调用它只是为了「配置改了 → 运行时跟着改」只有一条路径）；
/// - 四个 BLE 专用命令（`set_ble_auto_connect` / `set_ble_active_device` /
///   `add_ble_device` / `remove_ble_device`）。
pub fn sync_from_config(app: &AppHandle) {
    let config = app
        .state::<AppState>()
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let state = app.state::<BleState>();

    // 与状态里的配置镜像比对：镜像跨会话保留，因此这里能看出「这一次谁变了」。
    let (auto_toggled, address_changed) = {
        let inner = state.read();
        (
            inner.auto_connect != config.ble_auto_connect,
            inner.active_address != config.ble_active_address,
        )
    };

    // 先落镜像：快照（`get_ble_state` / `ble:state`）据此渲染设备列表与开关状态。
    if state.apply_preferences(&config) {
        state.broadcast(app);
    }

    // 切换自动连接开关一律清掉「用户刚主动断开」的抑制标志：
    // 打开开关就是想让它连；关掉开关也不会误连（看门狗紧接着就被停掉）。
    if auto_toggled && state.clear_suppressed() {
        state.broadcast(app);
    }

    // 目标变了：旧目标的链路不该再保持（否则「正在连的」与配置里的目标互相矛盾）。
    // 这条判定**在开关判定之前**：契约里「换目标」是独立语义，与开关状态无关；
    // 「关掉开关」则只停看门狗、不动已有链路。
    if address_changed {
        stop_session(app);
    }

    if !config.ble_auto_connect {
        // 关掉自动连接：停看门狗，**不动已有链路**（用户可能正戴着听歌）。
        if state.stop_watch() {
            state.broadcast(app);
        }
        return;
    }

    // 有目标就确保看门狗在跑；没有目标（或刚被抑制）就不该有 —— 它是「盯目标」的东西。
    if state.watch_wanted() {
        start_watch(app);
    } else if state.stop_watch() {
        state.broadcast(app);
    }
}

/// `set_config` 收到**整份**配置时，把三个 BLE 键换回内存里的权威值（其余字段不动）。
///
/// 为什么必须这么做：`prism3` 有主窗口与快捷小窗两个 WebView，各自持有一份配置副本，
/// 而 `set_config` 回写的是整份配置。若整份回写能改 BLE 键，那么「主窗口改主题」
/// 这种操作就会用**过期副本**覆盖掉快捷小窗刚写下的活动设备 / 设备列表（典型 lost
/// update）。先例是 `set_quick_panel_dock`：灰条的位置也只走专用命令，不走整份配置。
/// 因此 BLE 三个键**只**由 `set_ble_auto_connect` / `set_ble_active_device` /
/// `add_ble_device` / `remove_ble_device` 写。
pub fn preserve_config_keys(incoming: &mut AppConfig, authoritative: &AppConfig) {
    incoming.ble_auto_connect = authoritative.ble_auto_connect;
    incoming.ble_active_address = authoritative.ble_active_address.clone();
    incoming.ble_devices = authoritative.ble_devices.clone();
}

/// 起看门狗（幂等）：只有自动连接开着、有目标、且未被抑制时才真的起线程。
///
/// 条件检查放在这里（而不是每个调用方），这样任何调用路径都不可能违反
/// 「看门狗只在自动连接开着且有目标时活动」这条不变量。
/// 线程**只在这里创建**，退出条件只有「被 `stop_watch` 作废」——因此不会出现
/// 「刚判定该退出、同时又被要求起一轮」而丢掉看门狗的空窗（见 `watch_loop`）。
fn start_watch(app: &AppHandle) {
    let state = app.state::<BleState>();
    if !state.watch_wanted() {
        return;
    }
    let Some(generation) = state.begin_watch() else {
        return;
    };
    state.broadcast(app);

    let app = app.clone();
    std::thread::spawn(move || watch_loop(&app, generation));
}

/// 换会话目标：先停旧会话，等它释放耳机槽位之后再起新会话。
///
/// 命令必须立刻返回，所以等待放在独立线程里：会话线程收到停止信号会退出并
/// `disconnect`，那一刻 `end_session` 会清掉出站通道 —— 那就是「槽位已释放」的信号。
fn switch_session_target(app: &AppHandle, target: String) {
    stop_session(app);

    let app = app.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + HANDOVER_TIMEOUT;
        while app.state::<BleState>().is_session_active() {
            if std::time::Instant::now() >= deadline {
                log::warn!("等待上一轮蓝牙会话收尾超时，放弃切换到 {target}");
                return;
            }
            std::thread::sleep(HANDOVER_POLL);
        }

        // 等待期间用户可能又点了断开 / 又换了目标：动手前再确认一次。
        let state = app.state::<BleState>();
        if state.is_suppressed() {
            log::debug!("切换蓝牙目标期间被用户断开，放弃连接到 {target}");
            return;
        }
        if state.read().active_address.as_deref() != Some(target.as_str()) {
            log::debug!("切换蓝牙目标期间目标又变了，放弃连接到 {target}");
            return;
        }
        if let Err(err) = start_session(&app) {
            log::warn!("切换到蓝牙目标 {target} 失败：{err}");
        }
    });
}

/// 看门狗要盯的目标地址（没有配置或地址解析失败时为 `None`）。
fn watch_target(app: &AppHandle) -> Option<BDAddr> {
    let address = app.state::<BleState>().read().active_address.clone()?;
    match address.parse::<BDAddr>() {
        Ok(target) => Some(target),
        Err(err) => {
            log::warn!("自动连接目标 {address} 解析失败：{err}");
            None
        }
    }
}

/// 看门狗线程主体：`WATCH_SCAN_WINDOW` 有界扫描 → 扫到就把连接交给会话线程 →
/// 没扫到就等 `WATCH_IDLE` 再来一轮。
///
/// 三条硬约束：
/// - **连接 / 连接中 / 退避重连中绝不扫描**（耳机一次只接受一个 LE 中央设备，
///   扫描会与链路争用适配器）；
/// - 只在「仍是当前轮次」时才动手，被 `stop_watch` 作废即退出；
/// - 只管「发现并交给会话线程」：连接、订阅、断线退避重连全走既有那条路径
///   （`start_session` → `establish` → `session_loop`），这里不复制任何一份。
///
/// 扫描期间**不改 `phase`**：看门狗不是会话，动阶段会让界面在「搜索中」与
/// 「自动连接监视中」之间反复跳；`watching` 字段就是给它专用的。
fn watch_loop(app: &AppHandle, generation: u64) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            log::error!("创建自动连接运行时失败：{err}");
            let state = app.state::<BleState>();
            if state.end_watch(generation) {
                state.broadcast(app);
            }
            return;
        }
    };

    runtime.block_on(async {
        loop {
            let state = app.state::<BleState>();
            if !state.watch_is_current(generation) {
                break;
            }
            // 已有会话（连接中 / 已连接 / 退避重连中）：绝不扫描，只等。
            if state.is_session_active() {
                tokio::time::sleep(WATCH_IDLE).await;
                continue;
            }
            // 被「用户主动断开」抑制：不重连，等抑制被清掉（再点连接 / 切开关）。
            if state.is_suppressed() {
                tokio::time::sleep(WATCH_IDLE).await;
                continue;
            }
            let Some(target) = watch_target(app) else {
                // 目标被清掉（用户删了设备）：不扫描，等配置再给出目标。
                tokio::time::sleep(WATCH_IDLE).await;
                continue;
            };

            match first_adapter().await {
                Ok(adapter) => match scan_for(&adapter, target, WATCH_SCAN_WINDOW).await {
                    Ok(Some(_)) => {
                        // 扫到之后再确认一次：期间可能被停止 / 换了目标 / 已经连上。
                        if state.watch_is_current(generation) && watch_target(app) == Some(target) {
                            log::info!("自动连接：已扫到 {target}，开始建立链路");
                            if let Err(err) = start_session(app) {
                                log::warn!("自动连接失败：{err}");
                            }
                        }
                    }
                    Ok(None) => log::debug!("自动连接：本轮未扫到 {target}"),
                    Err(err) => log::warn!("自动连接扫描失败：{err}"),
                },
                Err(err) => log::warn!("自动连接暂时不可用：{err}"),
            }

            // 重试节奏：扫到之后同样等满一轮再检查（让连接与退避重连接管，不叠加扫描）。
            tokio::time::sleep(WATCH_IDLE).await;
        }
        log::debug!("自动连接监视已结束");
    });

    // 收尾：只有仍是当前轮次时才清标记（被停止 / 取代时停止方已经清过）。
    let state = app.state::<BleState>();
    if state.end_watch(generation) {
        state.broadcast(app);
    }
}

/// 把若干帧投给会话线程，并**等每一帧的写回执**（不等设备应答，状态由事件推送）。
///
/// 为什么必须等：帧可能因为链路刚断、会话正在退避重连而被丢弃。这种情况下命令层若返回
/// `Ok`，前端不会弹任何提示，用户看到的就是「点了没反应、也没有报错」——真机日志里那一串
/// `链路重连中，忽略一条蓝牙命令` 正是这条路径。回执只说明「帧写出去没有」，不说明设备
/// 接受了设置（设备的回执由 `ble:state` 反映）。
pub async fn send_frames(app: &AppHandle, frames: &[Vec<u8>]) -> AppResult<()> {
    let state = app.state::<BleState>();
    for frame in frames {
        let (reply, receiver) = oneshot::channel();
        // 没有会话时 `enqueue` 直接报错（不是静默丢弃）。
        state.enqueue(Outgoing::Write {
            frame: frame.clone(),
            reply,
        })?;
        await_write_receipt(receiver, WRITE_ACK_TIMEOUT).await?;
    }
    Ok(())
}

/// 等一条写回执。三种结局各自明确：
/// - 回执 `Ok` → 帧确实写出去了；
/// - 回执 `Err`（写失败 / 重连期间被丢弃）→ 原样上抛，前端弹 toast；
/// - 超时或回执通道断开（会话线程直接退出）→ 报「没写成」。
///
/// **拿不准的时候绝不能返回 `Ok`**：宁可让用户看到一个「可能未发送」的错误并重试，
/// 也不要让他以为设置生效了。
async fn await_write_receipt(
    receiver: oneshot::Receiver<AppResult<()>>,
    timeout: Duration,
) -> AppResult<()> {
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(outcome)) => outcome,
        // 发送端被丢弃：会话线程已经退出（通道关闭），帧没有被处理。
        Ok(Err(_)) => Err(AppError::Other("耳机连接已断开，设置未发送".into())),
        Err(_) => Err(AppError::Other(
            "等待耳机写入回执超时，设置可能未发送（请重试）".into(),
        )),
    }
}

/// 读一次电量并等它回来（超时由会话线程兜底，命令层只 await 一次性通道）。
pub async fn read_battery(app: &AppHandle) -> AppResult<BatteryReading> {
    let (reply, receiver) = oneshot::channel();
    app.state::<BleState>().enqueue(Outgoing::Request {
        key: KEY_STATUS,
        idx: IDX_BATTERY,
        frame: battery_request(),
        reply,
    })?;

    let response = receiver
        .await
        .map_err(|_| AppError::Other("等待耳机应答超时".into()))??;
    decode_battery(&response.data, now_secs())
        .ok_or_else(|| AppError::Other("电量应答格式无法解析".into()))
}

/// 会话线程主体：一轮轮地建立链路。
///
/// 「建立失败」是最终结果（扫描本身有超时，重复重试只会刷屏，用户重新点连接即可）；
/// 「建立之后断开」才退避重连，直到被取消。
async fn session_loop(app: &AppHandle, generation: u64, mut outbox: UnboundedReceiver<Outgoing>) {
    let mut backoff = RECONNECT_MIN_BACKOFF;
    loop {
        if is_stale(app, generation) {
            break;
        }
        match run_once(app, generation, &mut outbox).await {
            Err(err) => {
                log::warn!("建立蓝牙链路失败：{err}");
                commit(app, generation, |inner| {
                    let changed = inner.set_phase(BlePhase::Error);
                    inner.set_error(Some(err.to_string())) || changed
                });
                break;
            }
            Ok(RoundOutcome::Stopped) => break,
            Ok(RoundOutcome::Dropped) => {
                log::info!("蓝牙链路断开，{backoff:?} 后重连");
                if !commit(app, generation, |inner| {
                    inner.set_phase(BlePhase::Connecting)
                }) {
                    break;
                }
                if !wait_backoff(app, generation, &mut outbox, backoff).await {
                    break;
                }
                backoff = (backoff * 2).min(RECONNECT_MAX_BACKOFF);
            }
        }
    }
    finish_session(app, generation);
}

/// 会话线程收尾：断开已经在上层做过，这里只落状态。
fn finish_session(app: &AppHandle, generation: u64) {
    let state = app.state::<BleState>();
    state.end_session(generation);
    // 出错结束的会话保留 Error 阶段与原因，让界面能显示；正常结束回到 idle。
    let failed = state.read().phase == BlePhase::Error;
    if failed {
        state.broadcast(app);
    } else {
        commit(app, generation, |inner| inner.set_phase(BlePhase::Idle));
    }
}

/// 退避等待。期间收到取消（`Stop` / 通道关闭）立即返回 `false`。
async fn wait_backoff(
    app: &AppHandle,
    generation: u64,
    outbox: &mut UnboundedReceiver<Outgoing>,
    wait: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if is_stale(app, generation) {
            return false;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return true,
            message = outbox.recv() => {
                if !drain_while_reconnecting(message) {
                    return false;
                }
            },
        }
    }
}

/// 处理一条在**重连等待期间**到达的出站消息；返回 `false` 表示整轮会话到此结束。
///
/// 重连期间不能悄悄丢掉用户命令：命令层会返回 `Ok`、前端不弹任何提示，用户只觉得
/// 「点了没反应」（真机日志里成串的 `链路重连中，忽略一条蓝牙命令` 就是这么来的）。
/// 因此每条被丢弃的命令都要**回一个明确错误**，`Write` 与 `Request` 一视同仁，
/// 否则前者的调用方只能一直等到超时、后者会一直挂着。
///
/// 这里**不重新排队**：发不出去的设置就当失败让用户重试，而不是重连后偷偷补发 ——
/// 用户可能已经改主意，而且每次写后面都跟一次读回，补发会把界面拽回一个他没要的状态。
fn drain_while_reconnecting(message: Option<Outgoing>) -> bool {
    match message {
        Some(Outgoing::Stop) | None => false,
        Some(Outgoing::Write { frame, reply }) => {
            log::info!("链路重连中，丢弃一条蓝牙写命令：{}", hex(&frame));
            let _ = reply.send(Err(AppError::Other(RECONNECT_DISCARDED.into())));
            true
        }
        Some(Outgoing::Request {
            key,
            idx,
            frame,
            reply,
        }) => {
            log::info!(
                "链路重连中，丢弃一条蓝牙请求 key={key:#04X} idx={idx:#04X}：{}",
                hex(&frame)
            );
            let _ = reply.send(Err(AppError::Other(RECONNECT_DISCARDED.into())));
            true
        }
    }
}

/// 跑一轮完整会话：建立链路 → 收帧 → 断开。
async fn run_once(
    app: &AppHandle,
    generation: u64,
    outbox: &mut UnboundedReceiver<Outgoing>,
) -> AppResult<RoundOutcome> {
    let mut link = establish(app, generation).await?;

    // 链路就绪：清掉重连期间可能残留的错误，让界面只看到「已连接」。
    if !commit(app, generation, |inner| {
        let error_changed = inner.set_error(None);
        let phase_changed = inner.set_phase(BlePhase::Ready);
        error_changed || phase_changed
    }) {
        disconnect(&link.peripheral).await;
        return Ok(RoundOutcome::Stopped);
    }

    // 记账：读一次 GATT Device Name，并把「最近连接时间 + 名字」写回配置。
    // 这一步**永远不失败**（名字读不到是正常现象），因此不影响会话。
    remember_link(app, generation, &link.peripheral).await;

    let outcome = pump(app, generation, &mut link, outbox).await;
    // 无论走哪条路径退出都必须断开：耳机一次只接受一个 LE 中央设备，
    // 留着链路会把官方 App（以及用户自己的下一次连接）挡在外面。
    disconnect(&link.peripheral).await;
    Ok(outcome)
}

/// 建立链路：选适配器 → 无过滤扫描挑目标 → 连接 → 发现服务 → 订阅通知。
///
/// 连接成功之后的任何失败都会先断开再上抛，绝不把链路留在半途。
async fn establish(app: &AppHandle, generation: u64) -> AppResult<Established> {
    let target = connect_target(app)?;

    // 新一轮尝试开始：顺手清掉上一轮留下的错误，避免界面同时显示「错误」与「连接中」。
    if !commit(app, generation, |inner| {
        let error_changed = inner.set_error(None);
        let phase_changed = inner.set_phase(BlePhase::Scanning);
        error_changed || phase_changed
    }) {
        return Err(AppError::Other("蓝牙会话已取消".into()));
    }

    let adapter = first_adapter().await?;

    let peripheral = find_peripheral(&adapter, target).await?;
    if is_stale(app, generation) {
        return Err(AppError::Other("蓝牙会话已取消".into()));
    }

    let name = advertised_name_of(&peripheral).await;
    let address = peripheral.address().to_string();
    if !commit(app, generation, |inner| {
        let connection_changed = inner.set_connection(Some(address.clone()), name.clone());
        let phase_changed = inner.set_phase(BlePhase::Connecting);
        connection_changed || phase_changed
    }) {
        return Err(AppError::Other("蓝牙会话已取消".into()));
    }

    // 连接由**用户显式触发**或**看门狗扫到目标之后**走到这里；
    // 失败是普通错误，上报给界面即可，不是 panic。
    peripheral
        .connect_with_timeout(CONNECT_TIMEOUT)
        .await
        .map_err(|err| AppError::Other(format!("连接蓝牙耳机 {target} 失败：{err}")))?;

    match subscribe(&peripheral).await {
        Ok(link) => Ok(link),
        Err(err) => {
            disconnect(&peripheral).await;
            Err(err)
        }
    }
}

/// 发现服务 → 定位两个特征 → 订阅通知。失败时由调用方负责断开。
async fn subscribe(peripheral: &Peripheral) -> AppResult<Established> {
    peripheral
        .discover_services_with_timeout(DISCOVERY_TIMEOUT)
        .await
        .map_err(|err| AppError::Other(format!("发现耳机服务失败：{err}")))?;

    let characteristics = peripheral.characteristics();
    // 两个特征都限定在目标服务内：UUID 相同的特征可能出现在别的服务里，
    // 只按特征 UUID 找会订阅到不相干的通道。
    let write = characteristics
        .iter()
        .find(|item| item.service_uuid == SERVICE_UUID && item.uuid == WRITE_CHARACTERISTIC_UUID)
        .cloned()
        .ok_or_else(|| {
            AppError::Other(format!(
                "服务 {SERVICE_UUID} 未提供写入特征 {WRITE_CHARACTERISTIC_UUID}"
            ))
        })?;
    let notify = characteristics
        .iter()
        .find(|item| item.service_uuid == SERVICE_UUID && item.uuid == NOTIFY_CHARACTERISTIC_UUID)
        .cloned()
        .ok_or_else(|| {
            AppError::Other(format!(
                "服务 {SERVICE_UUID} 未提供通知特征 {NOTIFY_CHARACTERISTIC_UUID}"
            ))
        })?;

    peripheral
        .subscribe(&notify)
        .await
        .map_err(|err| AppError::Other(format!("订阅耳机通知失败：{err}")))?;
    let notifications = peripheral
        .notifications()
        .await
        .map_err(|err| AppError::Other(format!("获取耳机通知流失败：{err}")))?;

    Ok(Established {
        peripheral: peripheral.clone(),
        write,
        notifications,
    })
}

/// 链路就绪后的记账：读一次 GATT Device Name，再把这台设备写进配置与快照。
///
/// 名字读不到（特征不存在 / 读取失败 / 读回空串）都是**正常现象**——耳机不广播名字，
/// 真名只在 GATT Device Name 特征里，缺了就当没读到，绝不因此中断会话。
async fn remember_link(app: &AppHandle, generation: u64, peripheral: &Peripheral) {
    let address = peripheral.address().to_string();
    let name = read_device_name(peripheral).await;
    if let Some(name) = name.as_deref() {
        // 会话里的名字立即生效（配置镜像在同一函数里随后刷新）。
        commit(app, generation, |inner| {
            inner.set_name(Some(name.to_string()))
        });
    }
    remember_connection(app, &address, name.as_deref());
}

/// 读 GATT Device Name（`00002a00-…`）。
///
/// 该特征通常位于标准 GAP 服务 `00001800-…`，但固件也可能把它塞进别的服务，
/// 因此**跨全部已发现服务**按特征 UUID 查找（与 `subscribe` 里限定厂商服务的做法相反：
/// 那两个特征必须限定服务，否则会订阅到不相干的通道；这个名字特征则哪里都认）。
/// 找不到 / 读失败 / 解码为空一律返回 `None`，由调用方保持原样。
async fn read_device_name(peripheral: &Peripheral) -> Option<String> {
    let characteristic = peripheral
        .characteristics()
        .into_iter()
        .find(|item| item.uuid == DEVICE_NAME_CHARACTERISTIC_UUID)?;
    match peripheral.read(&characteristic).await {
        Ok(bytes) => decode_device_name(&bytes),
        Err(err) => {
            log::debug!("读取耳机设备名失败（不影响会话）：{err}");
            None
        }
    }
}

/// 解码 GATT Device Name：UTF-8 + 去掉**尾部** NUL 与空白，空串视为「没读到」。
///
/// 用有损解码：名字里出现非法字节也只是显示成替换字符，绝不让整轮会话失败。
fn decode_device_name(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed =
        text.trim_end_matches(|character: char| character == '\0' || character.is_whitespace());
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 把「这台设备连上了」写进配置与快照：活动目标 + 最近连接时间 +（读到的）缓存名。
///
/// 落盘是必须的：重启后 `ble_active_address` / `ble_devices` 就是「连接上次已连接的」
/// 的全部依据。写入失败只记日志——内存态已经更新，不影响本次会话。
fn remember_connection(app: &AppHandle, address: &str, name: Option<&str>) {
    let config = update_config(app, |config| {
        config.ble_active_address = Some(address.to_string());
        let entry = upsert_device(config, address);
        entry.last_connected_at = Some(now_secs());
        if let Some(name) = name {
            // 只有真的读到非空名字才覆盖缓存（空串绝不覆盖已有名字）。
            entry.name = Some(name.to_string());
        }
    });

    // 配置变了：快照镜像跟着刷新并广播（`watching` / `devices` 都取自镜像）。
    let state = app.state::<BleState>();
    if state.apply_preferences(&config) {
        state.broadcast(app);
    }
}

/// 读-改-写配置：**读、改、落盘都在同一把写锁内**，避免与 `set_config` 互相覆盖
/// （worker 拿着旧快照写盘会把用户刚存的设置抹掉）。
fn update_config<F>(app: &AppHandle, mutate: F) -> AppConfig
where
    F: FnOnce(&mut AppConfig),
{
    let state = app.state::<AppState>();
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    mutate(&mut guard);
    let config = guard.clone();
    if let Err(err) = crate::config::save(app, &config) {
        log::warn!("保存蓝牙设备记录失败：{err}");
    }
    config
}

/// 在配置里 upsert 一台设备（地址不分大小写），返回它的可变引用。
pub fn upsert_device<'a>(config: &'a mut AppConfig, address: &str) -> &'a mut BleDeviceEntry {
    let position = config
        .ble_devices
        .iter()
        .position(|entry| entry.address.eq_ignore_ascii_case(address));
    match position {
        Some(position) => &mut config.ble_devices[position],
        None => {
            config.ble_devices.push(BleDeviceEntry {
                address: address.to_string(),
                ..BleDeviceEntry::default()
            });
            let last = config.ble_devices.len() - 1;
            &mut config.ble_devices[last]
        }
    }
}

/// 记住某台设备的自适应降噪开关（**纯函数**：只改内存里的配置）。
///
/// 设备从不上报这一位（见 `adaptive_anc_request`），所以它只能由我们自己记：
/// 存在设备条目里，随配置落盘，换耳机时各看各的。条目不存在就新建
/// （只会写上地址 + 这一项，名字与最近连接时间留给别的路径填）。
pub fn remember_adaptive_anc(config: &mut AppConfig, address: &str, enabled: bool) {
    upsert_device(config, address).adaptive_anc = enabled;
}

/// 「当前设备」的地址：连接中的设备优先，其次是当前 / 上次连接的目标。
///
/// 与 `Inner::current_device` 同一口径：本地记忆类偏好（自适应降噪）就记在这台设备上。
pub fn current_device_address(app: &AppHandle) -> Option<String> {
    let state = app.state::<BleState>();
    let address = state.read().current_device().map(str::to_string);
    address
}

/// 收帧主循环：通知流 + 命令通道 + 心跳（电量轮询 / 应答超时）。
async fn pump(
    app: &AppHandle,
    generation: u64,
    link: &mut Established,
    outbox: &mut UnboundedReceiver<Outgoing>,
) -> RoundOutcome {
    let Established {
        peripheral,
        write,
        notifications,
    } = link;

    // 就绪后先把全部设置读一遍，让界面尽快有值（之后只有电量会被轮询）。
    for frame in refresh_requests() {
        if let Err(err) = peripheral.write(write, &frame, WRITE_TYPE).await {
            log::warn!("读取耳机初始状态失败：{err}");
            return RoundOutcome::Dropped;
        }
    }

    let mut pending: Vec<Pending> = Vec::new();
    let mut next_poll = tokio::time::Instant::now() + BATTERY_POLL_INTERVAL;

    loop {
        if is_stale(app, generation) {
            return RoundOutcome::Stopped;
        }
        tokio::select! {
            // 心跳：回收超时请求，并到点轮询一次电量。
            _ = tokio::time::sleep(TICK) => {
                let now = tokio::time::Instant::now();
                expire_pending(&mut pending, now);
                if now >= next_poll {
                    next_poll = now + BATTERY_POLL_INTERVAL;
                    if let Err(err) = peripheral.write(write, &battery_request(), WRITE_TYPE).await {
                        log::warn!("轮询电量失败：{err}");
                        return RoundOutcome::Dropped;
                    }
                }
            }
            message = outbox.recv() => match message {
                // 通道关闭 / 收到停止信号：用户断开，整轮会话结束。
                Some(Outgoing::Stop) | None => return RoundOutcome::Stopped,
                Some(Outgoing::Write { frame, reply }) => {
                    if let Err(err) = peripheral.write(write, &frame, WRITE_TYPE).await {
                        log::warn!("写入耳机失败：{err}");
                        // 把失败原因回给调用方（而不是让它等到超时）：前端据此弹提示。
                        let _ = reply.send(Err(AppError::Other(format!("写入耳机失败：{err}"))));
                        return RoundOutcome::Dropped;
                    }
                    // **帧确实写出去了**：回执成功。这个回执只说明「写出去了」，
                    // 不说明设备接受了设置（设备的回执由 `ble:state` 反映）。
                    let _ = reply.send(Ok(()));
                    // DEBUG：记下「我们要求什么」，与后面那条「收到应答」配对（读帧不记）。
                    log_setting_write(&frame);
                    // 写请求后面必须补一次同 key/idx 的读：设备对写只回单字节 ACK，
                    // 真值只能读回来（理由与实测的回复顺序见 `write_readback`）。
                    // 读回失败不回错误给调用方：用户那帧已经发出去了，这只是状态刷新失败，
                    // 断线重连后本来就会整体重读一遍。
                    if let Some(readback) = write_readback(&frame) {
                        if let Err(err) = peripheral.write(write, &readback, WRITE_TYPE).await {
                            log::warn!("读回耳机设置失败：{err}");
                            return RoundOutcome::Dropped;
                        }
                    }
                }
                Some(Outgoing::Request { key, idx, frame, reply }) => {
                    if let Err(err) = peripheral.write(write, &frame, WRITE_TYPE).await {
                        let _ = reply.send(Err(AppError::Other(format!("写入耳机失败：{err}"))));
                        return RoundOutcome::Dropped;
                    }
                    // 写请求同样记一条（读请求会被 `log_setting_write` 自己跳过）。
                    log_setting_write(&frame);
                    // 同上：等待中的写请求也要补读，否则它的状态一样不会更新。
                    // 这一帧不必登记 `Pending`（没人 await 它）：它的应答会作为普通
                    // 通知走 `handle_notification` → `Inner::apply`。
                    if let Some(readback) = write_readback(&frame) {
                        if let Err(err) = peripheral.write(write, &readback, WRITE_TYPE).await {
                            let _ = reply.send(Err(AppError::Other(format!(
                                "读回耳机设置失败：{err}"
                            ))));
                            return RoundOutcome::Dropped;
                        }
                    }
                    pending.push(Pending {
                        key,
                        idx,
                        deadline: tokio::time::Instant::now() + RESPONSE_TIMEOUT,
                        reply,
                    });
                }
            },
            notification = notifications.next() => match notification {
                Some(value) => handle_notification(app, generation, &value, &mut pending),
                // 通知流结束 = 链路断了（被官方 App 抢走 / 走远 / 关机）。
                None => {
                    log::info!("耳机通知流已结束（链路断开）");
                    return RoundOutcome::Dropped;
                }
            },
        }
    }
}

/// 处理一条通知：先满足等待中的请求，再把应答并入状态。
fn handle_notification(
    app: &AppHandle,
    generation: u64,
    value: &ValueNotification,
    pending: &mut Vec<Pending>,
) {
    // 回声门控在 `parse_response` 里：耳机会把请求原样回显到 notify 特征，
    // 回显帧同样满足校验和规则，因此必须先按 `09 FF` 过滤掉。
    // 但「回显」与「真无法解析」是两回事：前者每写一次就会出现一条（预期内），
    // 后者才值得 debug 级记录 —— 否则日志会被自己的回声刷满。
    let Some(response) = parse_response(&value.value) else {
        if is_request_echo(&value.value) {
            log::trace!("忽略请求回显：{}", hex(&value.value));
        } else {
            log::debug!("忽略无法解析的蓝牙通知：{}", hex(&value.value));
        }
        return;
    };

    // DEBUG：设备**报回来**的东西 —— `apply` 消费的就是这一帧。与 `log_setting_write`
    // 那条「我们要求什么」配对看，就能判断设备认没认那次写（例如 `data=00 00` 而我们要的是
    // `00 01`）。这里同样**不下结论**，只给事实；读回机制若失灵，这一行根本不出现。
    if let Some(summary) = frame_summary(&value.value) {
        log::debug!("收到应答 {summary}");
    }

    if let Some(position) = pending
        .iter()
        .position(|item| item.key == response.key && item.idx == response.idx)
    {
        let item = pending.remove(position);
        // 命令可能已经超时返回、接收端已经没了，这里忽略发送失败。
        let _ = item.reply.send(Ok(response.clone()));
    }

    let state = app.state::<BleState>();
    if let Some((next, changed)) = state.commit(generation, |inner| inner.apply(&response)) {
        if changed {
            emit_state(app, &next);
        }
    }
}

/// 回收超时的挂起请求（计时放在会话线程：命令层只 await 一次性通道）。
fn expire_pending(pending: &mut Vec<Pending>, now: tokio::time::Instant) {
    let mut index = 0;
    while index < pending.len() {
        if pending[index].deadline <= now {
            let item = pending.remove(index);
            let _ = item
                .reply
                .send(Err(AppError::Other("等待耳机应答超时".into())));
        } else {
            index += 1;
        }
    }
}

/// 取第一个可用的蓝牙适配器。
///
/// 空列表是环境问题，不是「稍后会有」：明确报错，绝不 unwrap。
/// 适配器存在但蓝牙开关关着时，扫描只会空转；提前给出可诊断的原因
/// （只把确定的 `PoweredOff` 当失败：`Unknown` / 查询失败都继续尝试）。
async fn first_adapter() -> AppResult<Adapter> {
    let manager = Manager::new()
        .await
        .map_err(|err| AppError::Other(format!("初始化蓝牙失败：{err}")))?;
    let adapters = manager
        .adapters()
        .await
        .map_err(|err| AppError::Other(format!("枚举蓝牙适配器失败：{err}")))?;
    let Some(adapter) = adapters.into_iter().next() else {
        return Err(AppError::Other("未找到可用的蓝牙适配器".into()));
    };
    if let Ok(CentralState::PoweredOff) = adapter.adapter_state().await {
        return Err(AppError::Other("系统蓝牙未开启".into()));
    }
    Ok(adapter)
}

/// 无过滤扫描，最多 `timeout` 内按地址（+ 厂商数据兜底判据）挑出目标。
///
/// 耳机的广播里**既没有设备名也没有服务 UUID**，所以 `ScanFilter { services }`
/// 会把唯一的目标过滤掉（实测：过滤后一个设备都扫不到），只能不带过滤地扫再自己挑。
/// 连接路径（`find_peripheral`）与自动连接看门狗都走这一条，扫描策略只有一份。
async fn scan_for(
    adapter: &Adapter,
    target: BDAddr,
    timeout: Duration,
) -> AppResult<Option<Peripheral>> {
    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|err| AppError::Other(format!("启动蓝牙扫描失败：{err}")))?;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut found = None;
    loop {
        match pick_target(adapter, target).await {
            Ok(Some(peripheral)) => {
                found = Some(peripheral);
                break;
            }
            Ok(None) => {}
            Err(err) => log::debug!("枚举蓝牙设备失败（继续扫描）：{err}"),
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(SCAN_POLL_INTERVAL).await;
    }

    // 停止扫描：失败不该盖过「有没有扫到」这个更有用的事实。
    if let Err(err) = adapter.stop_scan().await {
        log::warn!("停止蓝牙扫描失败：{err}");
    }
    Ok(found)
}

/// 无过滤扫描直到出现目标设备；扫不到即明确报错（报出目标地址），不无限重试。
async fn find_peripheral(adapter: &Adapter, target: BDAddr) -> AppResult<Peripheral> {
    match scan_for(adapter, target, SCAN_TIMEOUT).await? {
        Some(peripheral) => Ok(peripheral),
        None => {
            log_observed(adapter).await;
            Err(AppError::Other(format!(
                "{} 秒内没有扫描到蓝牙耳机 {target}（请确认耳机在范围内，\
                 并先让官方 App 断开连接）",
                SCAN_TIMEOUT.as_secs()
            )))
        }
    }
}

/// 扫描附近的候选耳机（`scan_ble_devices` 的实现）。
///
/// **只返回厂商数据命中公司标识 `0x5452` 的设备**：耳机既不广播名字也不广播服务 UUID，
/// 不过滤的话列表就是一整屏无法辨认的 MAC 地址；只看厂商数据也就顺带覆盖了可能的
/// 其它同族设备。适配器不可用是错误，**空列表是正常结果**（附近没有耳机）。
///
/// `known` 是已知设备列表（配置镜像）：只用来打 `known` 标记与回落到缓存名，
/// 因此这里不需要 `AppHandle`，真机探针也能直接调用。
pub async fn scan_candidates(known: &[BleDeviceEntry]) -> AppResult<Vec<BleCandidate>> {
    let adapter = first_adapter().await?;

    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|err| AppError::Other(format!("启动蓝牙扫描失败：{err}")))?;

    let deadline = tokio::time::Instant::now() + CANDIDATE_SCAN_WINDOW;
    let mut candidates: Vec<BleCandidate> = Vec::new();
    loop {
        if let Err(err) = collect_candidates(&adapter, known, &mut candidates).await {
            // 单轮枚举失败不该中断整窗扫描：下一轮会重新枚举。
            log::debug!("枚举蓝牙设备失败（继续扫描）：{err}");
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(SCAN_POLL_INTERVAL).await;
    }

    if let Err(err) = adapter.stop_scan().await {
        log::warn!("停止蓝牙扫描失败：{err}");
    }

    // 稳定排序：信号强的在前，同强度按地址 —— UI 展示与测试断言都不会跳来跳去。
    candidates.sort_by(|left, right| {
        right
            .rssi
            .cmp(&left.rssi)
            .then_with(|| left.address.cmp(&right.address))
    });
    log::info!("扫描到 {} 个候选耳机设备", candidates.len());
    Ok(candidates)
}

/// 把当前已发现的候选设备并入结果（按地址去重，保留信号更强的那份快照）。
async fn collect_candidates(
    adapter: &Adapter,
    known: &[BleDeviceEntry],
    out: &mut Vec<BleCandidate>,
) -> btleplug::Result<()> {
    for peripheral in adapter.peripherals().await? {
        let Ok(Some(properties)) = peripheral.properties().await else {
            // 读不到广播快照的设备直接跳过：没有厂商数据就无法判定是不是耳机。
            continue;
        };
        let address = peripheral.address();
        let Some(candidate) = candidate_of(address, &properties, known) else {
            continue;
        };
        match out
            .iter_mut()
            .find(|item| item.address == candidate.address)
        {
            Some(existing) => {
                if candidate.rssi > existing.rssi {
                    *existing = candidate;
                }
            }
            None => out.push(candidate),
        }
    }
    Ok(())
}

/// 由广播快照构造候选项：**只认厂商数据命中 `0x5452` 的设备**。
///
/// - 这是**过滤**而不是身份判据：payload 的内容（含末尾 6 字节）不稳定，不能用来
///   认设备（实测记录见 `pick_target` 的注释）；
/// - `manufacturer_payload` 原样给出广播里实际存在的字节（小写十六进制），不补齐、不臆造；
/// - 名字优先用广播名（耳机没有），缺失时回落到配置里的缓存名 —— 列表里显示一个
///   能认出来的名字，正是「否则就是一屏匿名 MAC」要解决的问题；
/// - 平台没报告 rssi 时为 `0`（不猜成某个信号值）。
fn candidate_of(
    address: BDAddr,
    properties: &PeripheralProperties,
    known: &[BleDeviceEntry],
) -> Option<BleCandidate> {
    if !has_vendor_manufacturer_data(properties) {
        return None;
    }
    let payload = properties.manufacturer_data.get(&DEVICE_COMPANY_ID)?;
    let address = address.to_string();
    let cached = known
        .iter()
        .find(|entry| entry.address.eq_ignore_ascii_case(&address));
    Some(BleCandidate {
        name: advertised_name(properties).or_else(|| cached.and_then(|entry| entry.name.clone())),
        rssi: properties.rssi.unwrap_or(0),
        manufacturer_payload: hex_lower(payload),
        known: cached.is_some(),
        address,
    })
}

/// 在已发现的设备里挑出目标。
///
/// 判据**只有一条**：地址等于目标地址（`BDAddr` 比较，天然大小写不敏感）。
/// **不做任何名字匹配**：耳机不广播名字，缺失是正常现象。
///
/// 早先这里还有第二条判据「厂商数据 `companyId == 0x5452` 且 payload 末 6 字节等于
/// 目标地址」——**那条前提已被实测推翻**：同一台耳机（地址始终是
/// `12:02:03:04:7C:0F`）的 payload 尾巴先是 `120203047c0f`（恰好等于自身地址，
/// 也是当初误判的来源），后来稳定变成 `a1020306cafd`（约 30 秒内采样 8 次全同，
/// 不是轮换值），而那个尾巴地址根本不广播。也就是说 payload 尾巴不是身份信号，
/// 拿它当判据只会**永远不命中**（白写一行死代码），一旦哪天碰巧命中还会认错设备。
/// 地址是唯一可用的身份信号（实测耳机不改地址），**不要**再把厂商数据当身份判据加回来；
/// 厂商数据只用于「扫描候选列表」的过滤（见 `candidate_of`）。
async fn pick_target(adapter: &Adapter, target: BDAddr) -> btleplug::Result<Option<Peripheral>> {
    for peripheral in adapter.peripherals().await? {
        let address = peripheral.address();
        if address != target {
            continue;
        }
        let properties = match peripheral.properties().await {
            Ok(properties) => properties,
            Err(err) => {
                // 广播快照读不到不该影响命中：地址已经够了。
                log::debug!("读取广播属性失败 addr={address}：{err}");
                None
            }
        };

        let rssi = properties.as_ref().and_then(|properties| properties.rssi);
        let name = properties.as_ref().and_then(advertised_name);
        log::info!("扫描到目标耳机 addr={address} rssi={rssi:?} 广播名={name:?}");
        return Ok(Some(peripheral));
    }
    Ok(None)
}

/// 厂商数据的第二种用途（**只做过滤，不做身份**）：广播里有没有公司标识 `0x5452`。
///
/// `scan_ble_devices` 靠它把候选列表收敛到本厂商的设备上 —— 耳机既不广播名字也不广播
/// 服务 UUID，不过滤的话列表就是一整屏无法辨认的 MAC 地址。它**不能**用来认设备：
/// payload 的内容（包括末尾那 6 字节）不是稳定身份，见 `pick_target` 的注释。
fn has_vendor_manufacturer_data(properties: &PeripheralProperties) -> bool {
    properties
        .manufacturer_data
        .get(&DEVICE_COMPANY_ID)
        .is_some_and(|payload| !payload.is_empty())
}

/// 广播里的设备名（`local_name` 优先，退回 `advertisement_name`）。
///
/// 本耳机**不广播名字**（实测 `local_name = None`，真名只在连接后的 GATT
/// Device Name 特征里），所以这里几乎总是 `None`；缺失是正常现象，不是错误。
fn advertised_name(properties: &PeripheralProperties) -> Option<String> {
    properties
        .local_name
        .clone()
        .or_else(|| properties.advertisement_name.clone())
}

/// 取发现阶段那台设备的广播名（拿不到快照时返回 `None`）。
async fn advertised_name_of(peripheral: &Peripheral) -> Option<String> {
    match peripheral.properties().await {
        Ok(properties) => properties.as_ref().and_then(advertised_name),
        Err(err) => {
            log::debug!("读取目标耳机广播属性失败：{err}");
            None
        }
    }
}

/// 扫描超时时把「到底看到了什么」打到 info 日志：扫不到的排查全靠这一行。
async fn log_observed(adapter: &Adapter) {
    match adapter.peripherals().await {
        Ok(peripherals) => {
            let mut seen: Vec<String> = Vec::with_capacity(peripherals.len());
            for peripheral in &peripherals {
                let rssi = match peripheral.properties().await {
                    Ok(properties) => properties.and_then(|properties| properties.rssi),
                    Err(_) => None,
                };
                seen.push(format!("{} (rssi={rssi:?})", peripheral.address()));
            }
            log::info!(
                "蓝牙扫描超时，共发现 {} 个设备：{}",
                seen.len(),
                seen.join(", ")
            );
        }
        Err(err) => log::warn!("列举蓝牙设备失败：{err}"),
    }
}

/// 断开链路。失败只记日志：调用方已经在退出路径上，没有更好的补救手段。
async fn disconnect(peripheral: &Peripheral) {
    if let Err(err) = peripheral.disconnect().await {
        log::warn!("断开蓝牙耳机失败：{err}");
    }
}

/// 目标设备地址（内置默认）。广播里没有名字，地址是唯一稳定的主键。
fn target_address() -> AppResult<BDAddr> {
    DEVICE_ADDRESS
        .parse::<BDAddr>()
        .map_err(|err| AppError::Other(format!("设备地址 {DEVICE_ADDRESS} 解析失败：{err}")))
}

/// 本轮要连接的目标地址：配置里的活动目标优先，没有配置时回落到内置的耳机地址。
///
/// 活动目标是「该连谁」的唯一来源（用户从扫描列表里选一台就会写进去）；
/// 没有任何配置的老用户仍然按内置地址连，行为与加入自动连接之前一致。
fn connect_target(app: &AppHandle) -> AppResult<BDAddr> {
    match app.state::<BleState>().read().active_address.clone() {
        Some(address) => address
            .parse::<BDAddr>()
            .map_err(|err| AppError::Other(format!("蓝牙目标地址 {address} 解析失败：{err}"))),
        None => target_address(),
    }
}

/// 打出十六进制字节（日志里排查未识别帧用）。
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 小写十六进制（**不带分隔符**）：候选设备的厂商数据按这个形状交给前端。
fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::Theme;

    use super::*;

    // 真机抓到的报文（见协议笔记 §11.2 / §12 / §13 / §14），逐字节断言。
    //
    // 帧常量一律带 `FRAME_`：本模块 `use super::*`，而生产代码里有同名的**偏移常量**
    // （`BATTERY_LEFT_CHARGING` = data 偏移 5 / `BATTERY_RIGHT_CHARGING` = 6），
    // 同名 fixture 会静默遮蔽它们，让偏移看起来「变成了一个数组」。
    const BATTERY_REQUEST: [u8; 10] = [0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x02];
    /// 电量应答：左 100% / 右 93% / 仓 74%，只有左耳在充电。
    const BATTERY_FRAME_LEFT_CHARGING: [u8; 18] = [
        0x09, 0xFF, 0x00, 0x00, 0x01, 0x01, 0x01, 0x12, 0x00, 0x01, 0x01, 0x64, 0x5D, 0x01, 0x00,
        0x4A, 0x00, 0x2B,
    ];
    /// 电量应答：左 96% / 右 90% / 仓不可用（0xFF），两只都不在充电。
    const BATTERY_FRAME_NOTHING_CHARGING: [u8; 18] = [
        0x09, 0xFF, 0x00, 0x00, 0x01, 0x01, 0x01, 0x12, 0x00, 0x01, 0x01, 0x60, 0x5A, 0x00, 0x00,
        0xFF, 0x00, 0xD8,
    ];
    /// 噪声模式写帧（降噪）：也与「自适应降噪关」同形。
    const NOISE_ANC_WRITE: [u8; 14] = [
        0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x01, 0x00, 0x00, 0x00, 0x8D,
    ];
    /// 降噪梯度写帧（深度）。
    const GRADIENT_DEEP_WRITE: [u8; 11] = [
        0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x87, 0x0B, 0x00, 0x02, 0x90,
    ];
    /// 噪声模式应答（降噪）。
    const NOISE_ANC_RESPONSE: [u8; 14] = [
        0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x02, 0x0E, 0x00, 0x01, 0x00, 0x00, 0x00, 0x20,
    ];

    fn parse(bytes: &[u8]) -> Response {
        parse_response(bytes).expect("这一帧应当解析成功")
    }

    /// 按协议规则重算校验和（用于构造「校验和正确」的坏帧）。
    fn resign(mut frame: Vec<u8>) -> Vec<u8> {
        let last = frame.len() - 1;
        let value = checksum(&frame[..last]);
        frame[last] = value;
        frame
    }

    #[test]
    fn read_requests_are_byte_exact() {
        // 全部读请求都是「data 恒为 00」的 10 字节帧。
        assert_eq!(battery_request(), BATTERY_REQUEST);
        assert_eq!(read_request(0x01, 0x01), BATTERY_REQUEST);
        assert_eq!(
            read_request(KEY_EQ, IDX_EQ),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x02, 0x01, 0x0A, 0x00, 0x03]
        );
        assert_eq!(
            read_request(KEY_NOISE, IDX_NOISE_MODE),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x02, 0x0A, 0x00, 0x08]
        );
        assert_eq!(
            read_request(KEY_NOISE, IDX_GAME_MODE),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x03, 0x0A, 0x00, 0x09]
        );
        assert_eq!(
            read_request(KEY_NOISE, IDX_ANC_GRADIENT),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x07, 0x0A, 0x00, 0x0D]
        );
        assert_eq!(
            read_request(KEY_DUAL, IDX_DUAL_DEVICE),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x08, 0x02, 0x0A, 0x00, 0x0A]
        );
    }

    #[test]
    fn refresh_requests_cover_every_setting() {
        let frames = refresh_requests();
        assert_eq!(frames.len(), 6, "电量 + EQ + 模式 + 游戏 + 梯度 + 双设备");
        assert_eq!(frames[0], BATTERY_REQUEST);
        // 每一帧都必须是「合法读请求」：长度正确且校验和自洽。
        for frame in &frames {
            assert_eq!(frame.len(), 0x0A);
            assert_eq!(frame[7] as usize, frame.len());
            assert_eq!(frame[9], checksum(&frame[..9]));
            assert_eq!(&frame[..5], &REQUEST_PREFIX);
        }
    }

    #[test]
    fn noise_mode_frames_are_byte_exact() {
        // 四个分栏的 one-hot 子档位；四帧的校验和都是 0x8D（data 之和都是 1）。
        assert_eq!(noise_mode_request(NoiseMode::Anc), NOISE_ANC_WRITE);
        assert_eq!(
            noise_mode_request(NoiseMode::Normal),
            vec![
                0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x00, 0x01, 0x00, 0x00, 0x8D
            ]
        );
        assert_eq!(
            noise_mode_request(NoiseMode::Wind),
            vec![
                0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x00, 0x00, 0x01, 0x00, 0x8D
            ]
        );
        assert_eq!(
            noise_mode_request(NoiseMode::Transparency),
            vec![
                0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x00, 0x00, 0x00, 0x01, 0x8D
            ]
        );
    }

    #[test]
    fn adaptive_anc_is_a_noise_mode_sub_level() {
        // 自适应降噪是「降噪分栏」的子档：d1 = 2 开、d1 = 1 关（关 = 固定降噪）。
        assert_eq!(
            adaptive_anc_request(true),
            vec![
                0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x02, 0x00, 0x00, 0x00, 0x8E
            ]
        );
        assert_eq!(adaptive_anc_request(false), NOISE_ANC_WRITE);
    }

    #[test]
    fn switch_and_preset_frames_are_byte_exact() {
        // 游戏模式：0 = 开、1 = 关。
        assert_eq!(
            game_mode_request(true),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x83, 0x0B, 0x00, 0x00, 0x8A]
        );
        assert_eq!(
            game_mode_request(false),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x83, 0x0B, 0x00, 0x01, 0x8B]
        );
        // 双设备连接：0 = 关、1 = 开（与游戏模式极性相反）。
        assert_eq!(
            dual_device_request(false),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x08, 0x82, 0x0B, 0x00, 0x00, 0x8B]
        );
        assert_eq!(
            dual_device_request(true),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x08, 0x82, 0x0B, 0x00, 0x01, 0x8C]
        );
        // EQ：0 = HIFI、1 = POP、2 = ROCK。
        assert_eq!(
            eq_preset_request(EqPreset::Hifi),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x02, 0x81, 0x0B, 0x00, 0x00, 0x84]
        );
        assert_eq!(
            eq_preset_request(EqPreset::Pop),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x02, 0x81, 0x0B, 0x00, 0x01, 0x85]
        );
        assert_eq!(
            eq_preset_request(EqPreset::Rock),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x02, 0x81, 0x0B, 0x00, 0x02, 0x86]
        );
    }

    #[test]
    fn anc_gradient_frames_are_byte_exact() {
        assert_eq!(
            anc_gradient_request(AncGradient::Light),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x87, 0x0B, 0x00, 0x00, 0x8E]
        );
        assert_eq!(
            anc_gradient_request(AncGradient::Medium),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x87, 0x0B, 0x00, 0x01, 0x8F]
        );
        assert_eq!(anc_gradient_request(AncGradient::Deep), GRADIENT_DEEP_WRITE);
    }

    #[test]
    fn gradient_write_reasserts_the_current_mode() {
        // 写梯度会让耳机隐含切回「降噪」，所以后面必须紧跟一次当前模式。
        let frames = anc_gradient_frames(AncGradient::Deep, Some(NoiseMode::Wind));
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], GRADIENT_DEEP_WRITE);
        assert_eq!(
            frames[1],
            vec![
                0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x82, 0x0E, 0x00, 0x00, 0x00, 0x01, 0x00, 0x8D
            ],
            "风噪"
        );

        // 模式未知时退化为读回一次模式：至少让界面与设备重新对齐。
        let frames = anc_gradient_frames(AncGradient::Medium, None);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], anc_gradient_request(AncGradient::Medium));
        assert_eq!(frames[1], read_request(KEY_NOISE, IDX_NOISE_MODE));
    }

    #[test]
    fn write_requests_are_followed_by_a_read_of_the_same_setting() {
        // 实测：设备对写只回单字节 ACK（游戏模式 `06 87 0A 00`、EQ `02 81 0A 00` …），
        // 因此写完之后必须补一次同 key/idx 的读，快照才拿得到真值 —— 否则 UI 会一直
        // 显示旧值，直到别的原因触发一次读（「点完不动，重开小窗才对」）。
        //
        // 这里用的都是真机抓到的写帧，读回帧逐字节断言。
        let game = game_mode_request(true);
        assert_eq!(
            game,
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x83, 0x0B, 0x00, 0x00, 0x8A]
        );
        assert_eq!(
            write_readback(&game),
            Some(read_request(KEY_NOISE, IDX_GAME_MODE)),
            "游戏模式写帧的读回"
        );
        assert_eq!(
            write_readback(&game).expect("写帧必须补读"),
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x06, 0x03, 0x0A, 0x00, 0x09]
        );

        let eq = eq_preset_request(EqPreset::Pop);
        assert_eq!(
            eq,
            vec![0x08, 0xEE, 0x00, 0x00, 0x00, 0x02, 0x81, 0x0B, 0x00, 0x01, 0x85]
        );
        assert_eq!(
            write_readback(&eq),
            Some(read_request(KEY_EQ, IDX_EQ)),
            "EQ 写帧的读回"
        );

        // 推导只看帧本身（idx 最高位），不靠调用方传 key/idx，因此每一条写帧都自动覆盖：
        // 降噪梯度、双设备、噪声模式、自适应降噪（同形于噪声模式）都补读。
        for (frame, key, idx) in [
            (
                anc_gradient_request(AncGradient::Medium),
                KEY_NOISE,
                IDX_ANC_GRADIENT,
            ),
            (dual_device_request(true), KEY_DUAL, IDX_DUAL_DEVICE),
            (
                noise_mode_request(NoiseMode::Anc),
                KEY_NOISE,
                IDX_NOISE_MODE,
            ),
            (adaptive_anc_request(true), KEY_NOISE, IDX_NOISE_MODE),
        ] {
            assert_eq!(frame[IDX_OFFSET], idx | WRITE_FLAG, "写帧的 idx 带写标志位");
            assert_eq!(
                write_readback(&frame),
                Some(read_request(key, idx)),
                "写帧 {} 应当补一帧同 key/idx 的读",
                hex(&frame)
            );
        }
    }

    #[test]
    fn read_requests_are_not_followed_by_another_read() {
        // 读帧没有写标志位：初始刷新与电量轮询都不该被补读拖慢。
        assert!(write_readback(&battery_request()).is_none());
        for frame in refresh_requests() {
            assert!(
                write_readback(&frame).is_none(),
                "读帧不该再补读：{}",
                hex(&frame)
            );
        }

        // 畸形 / 过短的帧不 panic，也不补读。
        assert!(write_readback(&[]).is_none());
        assert!(write_readback(&[0x08]).is_none());
        assert!(write_readback(&REQUEST_PREFIX).is_none());
    }

    #[test]
    fn gradient_pair_expands_to_write_read_write_read() {
        // 与 `pump` 的展开一致：每个写帧后面跟一帧同 key/idx 的读。
        // 于是「梯度写 + 模式重发」这对帧发出去是「梯度写 → 梯度读 → 模式写 → 模式读」：
        // 先确认梯度，再把设备隐含改掉的模式恢复并确认。
        let frames = anc_gradient_frames(AncGradient::Medium, Some(NoiseMode::Wind));
        let expanded: Vec<Vec<u8>> = frames
            .iter()
            .flat_map(|frame| std::iter::once(frame.clone()).chain(write_readback(frame)))
            .collect();
        assert_eq!(expanded.len(), 4, "两帧写 + 两次读回");
        assert_eq!(expanded[0], anc_gradient_request(AncGradient::Medium));
        assert_eq!(expanded[1], read_request(KEY_NOISE, IDX_ANC_GRADIENT));
        assert_eq!(expanded[2], noise_mode_request(NoiseMode::Wind));
        assert_eq!(expanded[3], read_request(KEY_NOISE, IDX_NOISE_MODE));
    }

    #[test]
    fn request_echoes_are_classified_apart_from_unparseable_frames() {
        // 回显帧必须被 `parse_response` 挡掉（`09 FF` 门控），但它是**预期内**的噪音：
        // 日志分类靠 `is_request_echo` 把「自己的请求」与「真无法解析」分开，
        // 否则每写一次设置就刷一条 debug。
        for frame in refresh_requests() {
            assert!(parse_response(&frame).is_none(), "读请求的回显必须被挡掉");
            assert!(is_request_echo(&frame), "读请求是请求回显：{}", hex(&frame));
        }
        // 写请求的回显同样带 `08 EE` 前缀（且 idx 还带着写标志位）。
        for frame in [
            game_mode_request(true),
            eq_preset_request(EqPreset::Pop),
            anc_gradient_request(AncGradient::Deep),
            dual_device_request(false),
            adaptive_anc_request(true),
        ] {
            assert!(parse_response(&frame).is_none(), "写请求的回显必须被挡掉");
            assert!(is_request_echo(&frame), "写请求是请求回显：{}", hex(&frame));
        }

        // 合法应答与真正的垃圾帧都不是「请求回显」。
        assert!(!is_request_echo(&BATTERY_FRAME_LEFT_CHARGING));
        assert!(!is_request_echo(&[]));
        assert!(!is_request_echo(b"StarRing Ultra"));
        assert!(!is_request_echo(&[0x08, 0xFF]));
    }

    #[test]
    fn channel_uuids_match_the_protocol() {
        // 三个 UUID 直接决定能不能连上正确的一问一答通道，用字面量钉住。
        assert_eq!(
            SERVICE_UUID.to_string(),
            "011bf5da-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            WRITE_CHARACTERISTIC_UUID.to_string(),
            "00007777-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            NOTIFY_CHARACTERISTIC_UUID.to_string(),
            "00008888-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(DEVICE_ADDRESS, "12:02:03:04:7C:0F");
    }

    #[test]
    fn response_parses_with_length_and_checksum() {
        let response = parse(&BATTERY_FRAME_LEFT_CHARGING);
        assert_eq!(response.key, KEY_STATUS);
        assert_eq!(response.idx, IDX_BATTERY);
        assert_eq!(
            response.data,
            vec![0x00, 0x01, 0x01, 0x64, 0x5D, 0x01, 0x00, 0x4A, 0x00]
        );
        assert_eq!(response.data.len(), 9);

        let response = parse(&NOISE_ANC_RESPONSE);
        assert_eq!(response.key, KEY_NOISE);
        assert_eq!(response.idx, IDX_NOISE_MODE);
        assert_eq!(response.data, vec![0x00, 0x01, 0x00, 0x00, 0x00], "降噪");
    }

    #[test]
    fn echoed_requests_are_rejected() {
        // 耳机会把请求原样回显到 notify 特征，而回显帧的校验和也是自洽的，
        // 因此必须靠 `09 FF` 门控把 `08 EE` 帧挡掉。
        assert!(parse_response(&BATTERY_REQUEST).is_none());
        assert!(parse_response(&NOISE_ANC_WRITE).is_none());
        assert!(parse_response(&GRADIENT_DEEP_WRITE).is_none());
        // 回显帧的校验和确实是对的（说明「挡掉的不是坏帧」）。
        assert_eq!(BATTERY_REQUEST[9], checksum(&BATTERY_REQUEST[..9]));
        assert_eq!(NOISE_ANC_WRITE[13], checksum(&NOISE_ANC_WRITE[..13]));
    }

    #[test]
    fn frames_with_wrong_length_are_rejected() {
        // LEN 字节与真实长度不符（声称 0x0E，实际 18 字节）。
        let mut wrong = BATTERY_FRAME_LEFT_CHARGING.to_vec();
        wrong[LEN_OFFSET] = 0x0E;
        assert!(parse_response(&resign(wrong)).is_none());

        // LEN 比真实长度大（少了个字节）。
        let mut short = BATTERY_FRAME_LEFT_CHARGING.to_vec();
        short.pop();
        assert!(parse_response(&resign(short)).is_none());

        // LEN 小到装不下 data + 校验和。
        let mut tiny = BATTERY_FRAME_NOTHING_CHARGING.to_vec();
        tiny[LEN_OFFSET] = DATA_OFFSET as u8;
        assert!(parse_response(&resign(tiny)).is_none());

        // 帧太短，连 `key`/`idx`/`LEN` 都读不到。
        assert!(parse_response(&[0x09, 0xFF]).is_none());
        assert!(parse_response(&[0x09, 0xFF, 0x00, 0x00, 0x01, 0x01]).is_none());
    }

    #[test]
    fn frames_with_wrong_checksum_are_rejected() {
        let mut wrong = BATTERY_FRAME_LEFT_CHARGING.to_vec();
        let last = wrong.len() - 1;
        wrong[last] = wrong[last].wrapping_add(1);
        assert!(parse_response(&wrong).is_none());

        // 只差 1 的校验和同样必须挡掉：校验和是用来识别「设备真的答了」的。
        let mut wrong = NOISE_ANC_RESPONSE.to_vec();
        let last = wrong.len() - 1;
        wrong[last] ^= 0xFF;
        assert!(parse_response(&wrong).is_none());

        // 长度正确、头正确，但校验和是 0 —— 也不能放过。
        let mut wrong = NOISE_ANC_RESPONSE.to_vec();
        let last = wrong.len() - 1;
        wrong[last] = 0x00;
        assert!(parse_response(&wrong).is_none());
    }

    #[test]
    fn unrelated_bytes_are_rejected() {
        assert!(parse_response(&[]).is_none());
        assert!(parse_response(&[0x00]).is_none());
        assert!(parse_response(b"StarRing Ultra").is_none());
        // 头对了但长度字段全 0。
        assert!(parse_response(&[0x09, 0xFF, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00]).is_none());
    }

    #[test]
    fn battery_decodes_percentages() {
        let reading = parse(&BATTERY_FRAME_LEFT_CHARGING);
        let battery = decode_battery(&reading.data, 1_700_000_000).expect("电量应答应当解出读数");
        assert_eq!(battery.left, Some(100));
        assert_eq!(battery.right, Some(93));
        assert_eq!(battery.case, Some(74));
        assert_eq!(battery.read_at_secs, 1_700_000_000);
    }

    #[test]
    fn battery_unavailable_maps_to_none() {
        // 充电仓不在位时读数是 0xFF：必须映射为 None，而不是 0 或 255。
        let reading = parse(&BATTERY_FRAME_NOTHING_CHARGING);
        let battery = decode_battery(&reading.data, 0).expect("电量应答应当解出读数");
        assert_eq!(battery.left, Some(96));
        assert_eq!(battery.right, Some(90));
        assert_eq!(battery.case, None, "0xFF 是「不可用」而不是 255%");

        // 左耳也不可用时同样归 None，且不影响右耳。
        let mut data = vec![0x00, 0x01, 0x01, 0xFF, 0x5A, 0x00, 0x00, 0xFF, 0x00];
        let battery = decode_battery(&data, 0).expect("电量应答应当解出读数");
        assert_eq!(battery.left, None);
        assert_eq!(battery.right, Some(90));
        assert_eq!(battery.case, None);

        // 数据段被截断时直接判为无法解析，而不是 panic。
        data.truncate(BATTERY_CASE);
        assert!(decode_battery(&data, 0).is_none());
    }

    #[test]
    fn battery_charging_flags_decode_from_real_frames() {
        // 真机帧：只有左耳在充电（厂商界面把左侧画成了充电色）。
        let reading = parse(&BATTERY_FRAME_LEFT_CHARGING);
        let battery = decode_battery(&reading.data, 0).expect("电量应答应当解出读数");
        assert_eq!(battery.left_charging, Some(true), "d5 = 01");
        assert_eq!(battery.right_charging, Some(false), "d6 = 00");

        // 真机帧：两只都不在充电。
        let reading = parse(&BATTERY_FRAME_NOTHING_CHARGING);
        let battery = decode_battery(&reading.data, 0).expect("电量应答应当解出读数");
        assert_eq!(battery.left_charging, Some(false));
        assert_eq!(battery.right_charging, Some(false));

        // 0 / 1 之外的取值语义未知 → None（不猜）。
        let data = [0x00, 0x01, 0x01, 0x64, 0x5D, 0x02, 0xFF, 0x4A, 0x00];
        let battery = decode_battery(&data, 0).expect("电量应答应当解出读数");
        assert_eq!(battery.left_charging, None, "02 未知");
        assert_eq!(battery.right_charging, None, "FF 未知");
        // 未知的充电位不影响电量百分比。
        assert_eq!(battery.left, Some(100));
        assert_eq!(battery.case, Some(74));
    }

    #[test]
    fn response_idx_ignores_the_write_flag() {
        // 应答里的 idx 是否清掉 0x80 并不一致，所以匹配一律按 idx & 0x7F。
        // 梯度写帧的回显索引仍是 0x87。
        let mut frame = NOISE_ANC_RESPONSE.to_vec();
        frame[IDX_OFFSET] = 0x82;
        let response = parse(&resign(frame));
        assert_eq!(response.idx, IDX_NOISE_MODE, "0x82 → 0x02");

        let mut frame = NOISE_ANC_RESPONSE.to_vec();
        frame[IDX_OFFSET] = 0x87;
        let response = parse(&resign(frame));
        assert_eq!(response.idx, IDX_ANC_GRADIENT, "0x87 → 0x07");
    }

    #[test]
    fn noise_mode_decodes_to_the_right_slot() {
        // 四个分栏：d1..d4 各自 one-hot。
        assert_eq!(
            decode_noise(&[0x00, 0x01, 0x00, 0x00, 0x00]),
            Some((NoiseMode::Anc, false))
        );
        assert_eq!(
            decode_noise(&[0x00, 0x02, 0x00, 0x00, 0x00]),
            Some((NoiseMode::Anc, true)),
            "d1 = 2 是自适应降噪"
        );
        assert_eq!(
            decode_noise(&[0x00, 0x00, 0x01, 0x00, 0x00]),
            Some((NoiseMode::Normal, false))
        );
        assert_eq!(
            decode_noise(&[0x00, 0x00, 0x00, 0x01, 0x00]),
            Some((NoiseMode::Wind, false))
        );
        assert_eq!(
            decode_noise(&[0x00, 0x00, 0x00, 0x00, 0x01]),
            Some((NoiseMode::Transparency, false))
        );
        // 全 0 是未知状态（不是某个模式）；长度不足同样判为未知。
        assert_eq!(decode_noise(&[0x00, 0x00, 0x00, 0x00, 0x00]), None);
        assert_eq!(decode_noise(&[0x00, 0x01]), None);
    }

    #[test]
    fn responses_are_applied_to_the_snapshot() {
        let mut inner = Inner::idle();

        // 电量：0xFF → None，同时刷新读数时间。
        assert!(inner.apply(&parse(&BATTERY_FRAME_NOTHING_CHARGING)));
        let battery = inner.battery.expect("电量应当已写入");
        assert_eq!(battery.left, Some(96));
        assert_eq!(battery.case, None);
        assert_eq!(battery.left_charging, Some(false));

        // 噪声模式：写 key=06 idx=02 后设备主动推送的新模式。
        // `adaptive_anc` **不在这里**：它现在是本地记忆，设备报文对它没有发言权
        // （见 `apply` 的注释与 `adaptive_anc_follows_local_memory`）。
        assert!(inner.apply(&parse(&NOISE_ANC_RESPONSE)));
        assert_eq!(inner.noise_mode, Some(NoiseMode::Anc));

        // 相同的应答再来一次：只有读数时间变了，`noise_mode` 不应重复广播。
        let before = inner.noise_mode;
        assert!(!inner.apply(&parse(&NOISE_ANC_RESPONSE)));
        assert_eq!(inner.noise_mode, before);

        // EQ → ROCK（08 FF … 02 86 的应答形态）。
        let eq = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x02, 0x01, 0x0B, 0x00, 0x02, 0x00,
        ]);
        assert!(inner.apply(&parse(&eq)));
        assert_eq!(inner.eq_preset, Some(EqPreset::Rock));

        // 游戏模式关（0 = 开、1 = 关）。
        let game = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x03, 0x0B, 0x00, 0x01, 0x00,
        ]);
        assert!(inner.apply(&parse(&game)));
        assert_eq!(inner.game_mode, Some(false));

        // 降噪梯度深度。
        let gradient = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x07, 0x0B, 0x00, 0x02, 0x00,
        ]);
        assert!(inner.apply(&parse(&gradient)));
        assert_eq!(inner.anc_gradient, Some(AncGradient::Deep));

        // 双设备连接开（0 = 关、1 = 开）。
        let dual = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x08, 0x02, 0x0B, 0x00, 0x01, 0x00,
        ]);
        assert!(inner.apply(&parse(&dual)));
        assert_eq!(inner.dual_device, Some(true));

        // 不认识 key/idx 的应答不该改动任何字段。
        let unknown = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x0A, 0x01, 0x0B, 0x00, 0x00, 0x00,
        ]);
        assert!(!inner.apply(&parse(&unknown)));
    }

    #[test]
    fn snapshot_field_names_match_the_frozen_contract() {
        // 前端按这些字段名写，改名即破坏契约 —— 用序列化结果钉住。
        let snapshot = dto(&Inner {
            phase: BlePhase::Ready,
            address: Some(DEVICE_ADDRESS.to_string()),
            name: Some("StarRing Ultra".into()),
            battery: Some(BatteryReading {
                left: Some(95),
                right: Some(89),
                case: None,
                left_charging: Some(true),
                right_charging: Some(false),
                read_at_secs: 42,
            }),
            noise_mode: Some(NoiseMode::Wind),
            anc_gradient: Some(AncGradient::Deep),
            eq_preset: Some(EqPreset::Hifi),
            game_mode: Some(false),
            dual_device: Some(true),
            devices: vec![BleDeviceEntry {
                address: DEVICE_ADDRESS.to_string(),
                name: Some("StarRing Ultra_BLE".into()),
                last_connected_at: Some(1_700_000_000),
                // `adaptiveAnc` 现在**只**来自这里（本地记忆），下面断言它被取了出来。
                adaptive_anc: true,
            }],
            active_address: Some(DEVICE_ADDRESS.to_string()),
            auto_connect: true,
            session_target: Some(DEVICE_ADDRESS.to_string()),
            // 看门狗「在跑」，但阶段是 `ready`：`watching` 仍然是 `false`
            // —— 它证明该字段是派生值，而不是配置的照抄。
            watcher_running: true,
            suppressed: false,
            watch_generation: 1,
            error: None,
        });
        let value = serde_json::to_value(&snapshot).expect("快照应当可序列化");
        let object = value.as_object().expect("快照应当是对象");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "activeAddress",
                "adaptiveAnc",
                "address",
                "ancGradient",
                "autoConnect",
                "battery",
                "devices",
                "dualDevice",
                "eqPreset",
                "error",
                "gameMode",
                "name",
                "noiseMode",
                "phase",
                "watching",
            ]
        );

        // 枚举与阶段一律 camelCase（前端按字面量比对）。
        assert_eq!(value["phase"], "ready");
        assert_eq!(value["noiseMode"], "wind");
        assert_eq!(value["ancGradient"], "deep");
        assert_eq!(value["eqPreset"], "hifi");

        // 配置镜像类的字段：值直接来自镜像，`watching` / `adaptiveAnc` 则是派生值。
        assert_eq!(value["activeAddress"], DEVICE_ADDRESS);
        assert_eq!(value["autoConnect"], true);
        assert_eq!(
            value["watching"], false,
            "ready 阶段没有看门狗在轮询：watching 是派生值而不是配置的照抄"
        );
        assert_eq!(
            value["adaptiveAnc"], true,
            "adaptiveAnc 是**本地记忆**：来自 devices 里那台设备的条目，而不是设备报文"
        );

        // 设备项的键名（前端按 `lastConnectedAt` 读缓存时间）。
        // 注意 `adaptiveAnc` **不在**设备项里：它是**当前设备**的本地记忆，只出现在快照
        // 顶层（上面那条断言）；设备项的形状保持既有契约不变。
        let device = value["devices"][0].as_object().expect("设备项应当是对象");
        let mut device_keys: Vec<&str> = device.keys().map(String::as_str).collect();
        device_keys.sort_unstable();
        assert_eq!(device_keys, vec!["address", "lastConnectedAt", "name"]);
        assert_eq!(device["name"], "StarRing Ultra_BLE");

        let battery = value["battery"].as_object().expect("电量应当是对象");
        let mut battery_keys: Vec<&str> = battery.keys().map(String::as_str).collect();
        battery_keys.sort_unstable();
        assert_eq!(
            battery_keys,
            vec![
                "case",
                "left",
                "leftCharging",
                "readAtSecs",
                "right",
                "rightCharging",
            ]
        );
    }

    #[test]
    fn session_generation_gates_stale_writers() {
        let state = BleState::new();

        // 没有会话时不允许投递（明确报错，而不是静默丢弃）。
        assert!(state.enqueue(Outgoing::Stop).is_err());

        let (generation, _receiver) = state
            .begin_session(Some(DEVICE_ADDRESS.to_string()))
            .expect("首轮会话应当可以开始");
        assert!(state
            .commit(generation, |inner| inner.set_phase(BlePhase::Scanning))
            .is_some());

        // 已在会话中时再来一轮是空操作（幂等）。
        assert!(state.begin_session(None).is_none());

        // 用户断开：立刻回到 idle，且旧代号从此不再被接受。
        state.request_cancel();
        assert!(state.is_stale(generation));
        assert!(state
            .commit(generation, |inner| inner.set_phase(BlePhase::Ready))
            .is_none());
        assert_eq!(state.snapshot().phase, BlePhase::Idle);
        assert_eq!(state.snapshot().battery, None, "断开后清掉旧读数");

        // 会话线程收尾后即可再次连接。
        state.end_session(generation);
        assert!(state.begin_session(None).is_some());
    }

    /// 手动冒烟测试：验证「新建线程 + current-thread 运行时」能真的开出蓝牙适配器。
    ///
    /// btleplug 不显式初始化 COM 单元，而会话线程是裸 `std::thread`（没有继承主线程的
    /// 单元）——这条链路只有真机才能验证，所以默认 `#[ignore]`：
    /// `cargo test --lib -- --ignored ble_adapter_smoke`。
    /// 只枚举适配器，不扫描、不连接（耳机一次只接受一个 LE 中央设备）。
    #[test]
    #[ignore = "需要本机蓝牙硬件"]
    fn ble_adapter_smoke() {
        let probe = std::thread::spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建运行时失败");
            runtime.block_on(async {
                let manager = Manager::new().await.expect("初始化蓝牙失败");
                let adapters = manager.adapters().await.expect("枚举适配器失败");
                println!("适配器数量 = {}", adapters.len());
                for adapter in &adapters {
                    println!("  info = {:?}", adapter.adapter_info().await);
                    println!("  state = {:?}", adapter.adapter_state().await);
                    println!("  address = {:?}", adapter.adapter_address().await);
                }
                assert!(!adapters.is_empty(), "本机应当至少有一个蓝牙适配器");
            });
        });
        probe.join().expect("探针线程不应 panic");
    }

    /// 手动冒烟测试：验证修正后的扫描策略能真的扫到耳机（**只扫描，不连接**）。
    ///
    /// 这条用例守的是最容易写错的一环：耳机广播里没有名字、没有服务 UUID，
    /// 过滤扫描会一个设备都扫不到，只能无过滤扫描 + **按地址**挑（厂商数据只用于
    /// 候选列表过滤，**不是身份判据**——详见 `pick_target`）。
    /// 用 `cargo test --lib -- --ignored ble_scan_finds_the_earbud --nocapture` 跑。
    #[test]
    #[ignore = "需要耳机在范围内（只扫描，不连接）"]
    fn ble_scan_finds_the_earbud() {
        let probe = std::thread::spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建运行时失败");
            runtime.block_on(async {
                let manager = Manager::new().await.expect("初始化蓝牙失败");
                let adapters = manager.adapters().await.expect("枚举适配器失败");
                let adapter = adapters.into_iter().next().expect("本机应当有蓝牙适配器");
                let target = target_address().expect("设备地址应当可解析");
                let peripheral = find_peripheral(&adapter, target)
                    .await
                    .expect("应当扫到目标耳机");
                println!("命中 {}", peripheral.address());
                assert_eq!(peripheral.address(), target);
            });
        });
        probe.join().expect("探针线程不应 panic");
    }

    /// 手动探针：验证候选扫描**只挑出厂商数据命中 `0x5452` 的设备**，并如实上报广播内容。
    ///
    /// 只扫描、不连接（不占用耳机唯一的 LE 中央槽位），守的是那条过滤与候选字段本身：
    /// 耳机既不广播名字也不广播服务 UUID，不过滤的话列表就是一整屏匿名 MAC。
    /// 用 `cargo test --lib -- --ignored --nocapture ble_scan_lists_candidates` 跑。
    ///
    /// **一次实测记录（也是曾经写错判据的地方）**：同一台耳机、自身地址始终是
    /// `12:02:03:04:7C:0F`，厂商 payload 却先后出现过两种尾巴：
    ///
    /// - 早期：`00030001000b016400 120203047c0f`（尾巴**恰好等于**自身地址，当初据此误判
    ///   「payload 末 6 字节 = 设备地址」，`has_target_manufacturer_data` 就是这么来的）；
    /// - 现在：`00030001000b016400 a1020306cafd`（尾巴**不等于**自身地址；约 30 秒内采样
    ///   8 次全是这一个值，即**稳定但非本机地址**，而且那个尾巴地址根本不广播）。
    ///
    /// 尾巴的所有者**未证实**（看地址格式像同厂商的另一只/另一侧，但这只是猜测）。
    /// 结论：**payload 尾部不是身份信号**，谁都不要再用它认设备；身份只认地址。
    /// 因此本探针只断言「目标在列表里 + 字段如实」，不再断言 payload 的尾巴。
    ///
    /// 判据：
    ///
    /// - 目标地址**确实出现在候选里**（耳机在范围内时过滤不该把它滤掉）；
    /// - 每一条候选都带厂商数据：非空、偶数长度、**小写**十六进制，且就是实际观测到的字节；
    /// - 地址不重复；
    /// - `rssi` 被平台报告（约定 `0` = 未报告，见 `candidate_of`）；
    /// - `known` 与传入的配置镜像一致：镜像里没有 → `false`；镜像里有 → `true` 且有名字。
    #[test]
    #[ignore = "需要耳机在范围内（只扫描，不连接）"]
    fn ble_scan_lists_candidates() {
        let probe = std::thread::spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建运行时失败");
            runtime.block_on(async {
                let target = target_address().expect("设备地址应当可解析");
                assert_eq!(target.to_string(), DEVICE_ADDRESS, "地址是唯一身份信号");

                // 第一轮：已知列表为空 → 所有候选都必须是「未知设备」。
                let candidates = scan_candidates(&[]).await.expect("扫描不应当失败");
                println!("候选设备 {} 个（已知列表为空）：", candidates.len());
                for candidate in &candidates {
                    println!(
                        "  {} name={:?} rssi={} payload={} known={}",
                        candidate.address,
                        candidate.name,
                        candidate.rssi,
                        candidate.manufacturer_payload,
                        candidate.known
                    );
                }
                assert!(
                    !candidates.is_empty(),
                    "耳机在范围内时应当至少扫到一个 0x5452 设备"
                );
                let observed = candidates
                    .iter()
                    .find(|candidate| candidate.address == DEVICE_ADDRESS)
                    .unwrap_or_else(|| {
                        panic!(
                            "候选里必须有目标耳机 {DEVICE_ADDRESS}（它不广播名字/服务 UUID，\
                             只能靠地址与厂商标识认出来）"
                        )
                    });
                println!("目标耳机广播：{}", observed.manufacturer_payload);

                for candidate in &candidates {
                    assert!(
                        !candidate.manufacturer_payload.is_empty()
                            && candidate.manufacturer_payload.len() % 2 == 0,
                        "候选必须带厂商数据（实际观测到的字节）：{}",
                        candidate.address
                    );
                    assert!(
                        candidate
                            .manufacturer_payload
                            .chars()
                            .all(|character| character.is_ascii_hexdigit()
                                && !character.is_ascii_uppercase()),
                        "厂商数据必须是**小写**十六进制：{}",
                        candidate.manufacturer_payload
                    );
                    assert!(
                        candidate.rssi < 0,
                        "平台应当报告 rssi（约定 0 = 未报告）：{} rssi={}",
                        candidate.address,
                        candidate.rssi
                    );
                    assert!(
                        !candidate.known,
                        "已知列表为空时不该有候选被标成已知：{}",
                        candidate.address
                    );
                }

                let mut addresses: Vec<&str> = candidates
                    .iter()
                    .map(|candidate| candidate.address.as_str())
                    .collect();
                let total = addresses.len();
                addresses.sort_unstable();
                addresses.dedup();
                assert_eq!(addresses.len(), total, "同一次扫描里地址不该重复");

                // 第二轮：把目标放进已知列表（带缓存名）→ `known` 必须跟着变，
                // 且广播没名字时用缓存名兜底。
                let known = vec![BleDeviceEntry {
                    address: DEVICE_ADDRESS.to_string(),
                    name: Some("StarRing Ultra_BLE".into()),
                    last_connected_at: Some(7),
                    adaptive_anc: false,
                }];
                let candidates = scan_candidates(&known).await.expect("第二次扫描不应当失败");
                let observed = candidates
                    .iter()
                    .find(|candidate| candidate.address == DEVICE_ADDRESS)
                    .expect("第二次扫描同样应当看到目标耳机");
                assert!(observed.known, "地址已在已知列表里 → known 必须为 true");
                assert!(
                    observed.name.is_some(),
                    "至少应当回落到缓存名（耳机自己不广播名字）"
                );
                println!(
                    "已知设备回读：known={} name={:?}",
                    observed.known, observed.name
                );
            });
        });
        probe.join().expect("探针线程不应 panic");
    }

    /// 端到端探针等待电量应答的上限（连接与订阅另有 `CONNECT_TIMEOUT` /
    /// `DISCOVERY_TIMEOUT` 约束，因此整体是有界的）。
    const PROBE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

    /// 探针抓到的一帧电量应答：**未经改动的原始帧** + 解码后的读数 + 读到的设备名。
    ///
    /// 断言直接打在 `frame` 上，这样 `0xFF -> None` 之类的规则是被真正钉住的，
    /// 而不是「相信解码器做过」。
    struct ProbeReading {
        frame: Vec<u8>,
        battery: BatteryReading,
        /// GATT Device Name 的读取结果（`None` = 没读到，属正常现象）。
        name: Option<String>,
    }

    /// 从通知流里等一帧电量应答：**回声帧与非电量帧一律跳过**。
    ///
    /// 回显帧（`08 EE …`）与校验和 / 长度不符的帧都由 `parse_response` 挡掉，
    /// 这里只做「是不是我要的那一帧」的筛选 —— 与生产路径同一套门控。
    /// 一并把原始帧带出去，供判据在原始字节上复查。
    async fn await_battery_response(
        notifications: &mut Notifications,
    ) -> AppResult<(Vec<u8>, Response)> {
        loop {
            match notifications.next().await {
                Some(notification) => {
                    if let Some(response) = parse_response(&notification.value) {
                        if response.key == KEY_STATUS && response.idx == IDX_BATTERY {
                            return Ok((notification.value, response));
                        }
                    }
                }
                None => return Err(AppError::Other("通知流已结束".into())),
            }
        }
    }

    /// 探针主体：连接 → 发现服务 + 订阅通知 → 读设备名 → 写电量读请求 → 解码应答。
    ///
    /// **本函数不负责断开**：断开交给调用方，这样无论走到哪一步失败，
    /// 耳机唯一的 LE 中央槽位都会被释放。走的是与生产路径同一批函数
    /// （`subscribe` / `read_device_name` 即 `establish` 与 `remember_link` 里用的那些）。
    async fn probe_read_battery(peripheral: &Peripheral) -> AppResult<ProbeReading> {
        peripheral
            .connect_with_timeout(CONNECT_TIMEOUT)
            .await
            .map_err(|err| AppError::Other(format!("连接失败：{err}")))?;

        let mut link = subscribe(peripheral).await?;

        // 设备名：与生产路径同一个函数（跨全部服务找 `00002a00-…`）。
        // 读不到是正常现象，因此这里不当作失败。
        let name = read_device_name(&link.peripheral).await;

        link.peripheral
            .write(&link.write, &battery_request(), WRITE_TYPE)
            .await
            .map_err(|err| AppError::Other(format!("写入电量请求失败：{err}")))?;

        let (frame, response) = tokio::time::timeout(
            PROBE_RESPONSE_TIMEOUT,
            await_battery_response(&mut link.notifications),
        )
        .await
        .map_err(|_| AppError::Other("等待电量应答超时".into()))??;

        println!("原始应答帧：{}", hex(&frame));

        let battery = decode_battery(&response.data, now_secs())
            .ok_or_else(|| AppError::Other("电量应答无法解析".into()))?;

        Ok(ProbeReading {
            frame,
            battery,
            name,
        })
    }

    /// 手动端到端探针：连接 → 发现服务 → 订阅通知 → 读电量 → 解码。
    ///
    /// 守的是**接线之后**那条腿（写入 + 回声过滤 + 长度/校验 + 解码），也是唯一
    /// 只有真机才能验证的一环。它会短暂占用耳机**唯一**的 LE 中央槽位，
    /// 因此无论成败都在断言之前断开（官方 App 走经典 SPP，不受影响）。
    /// 用 `cargo test --lib -- --ignored --nocapture ble_reads_the_battery` 跑。
    ///
    /// **接受判据（按设备真实状态定的）**：某只耳机睡着 / 在充电仓里时，设备会把该侧
    /// 字段报成 `0xFF`（按契约 = 不可用 → `None`），这是**正常状态**而不是故障。
    /// 因此本用例要求：
    /// - (a) 确实收到了一条通过**生产级** `parse_response` 的应答（`09 FF` 回声门控 +
    ///   `LEN` + 校验和），并在原始帧上把这三条各再钉一遍；
    /// - (b) 左 / 右至少一只报出 `1..=100` 的电量；
    /// - (c) 所有出现的百分比（含充电仓）都落在 `0..=100`；
    /// - (d) `0xFF -> None` 真的成立：某槽位为 `None` 时，原始帧里对应的数据字节必须是
    ///   `0xFF`（充电位语义不同：`None` 表示原始字节既不是 `0` 也不是 `1`）。
    ///
    /// 于是**失败只可能意味着链路或解析坏了**，而不再会因为「某只耳机在仓里」误报。
    /// 真机实测的一帧（左耳不通信、右耳 87%）：
    /// `key=0x01 idx=0x01 data=00 01 00 FF 57 FF 00 FF 00`。
    #[test]
    #[ignore = "需要耳机在范围内（会短暂占用唯一的 LE 连接）"]
    fn ble_reads_the_battery() {
        let probe = std::thread::spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建运行时失败");
            runtime.block_on(async {
                let manager = Manager::new().await.expect("初始化蓝牙失败");
                let adapters = manager.adapters().await.expect("枚举适配器失败");
                let adapter = adapters.into_iter().next().expect("本机应当有蓝牙适配器");
                let target = target_address().expect("设备地址应当可解析");
                let peripheral = find_peripheral(&adapter, target)
                    .await
                    .expect("应当扫到目标耳机");

                let outcome = probe_read_battery(&peripheral).await;
                // 先无条件断开，再做断言：断言失败也不能把 LE 槽位留在占用状态。
                disconnect(&peripheral).await;

                let reading = outcome.expect("应当收到一条通过门控 / 长度 / 校验和的电量应答");
                let ProbeReading {
                    frame,
                    battery,
                    name,
                } = reading;
                println!(
                    "电量读数：左={:?} 右={:?} 仓={:?} 左充={:?} 右充={:?} readAtSecs={}",
                    battery.left,
                    battery.right,
                    battery.case,
                    battery.left_charging,
                    battery.right_charging,
                    battery.read_at_secs
                );
                // 设备名：读不到是**正常现象**（耳机不广播名字、固件也可能不提供该特征），
                // 因此只在「读到了」的时候检查它是不是一个干净的名字。
                println!("GATT 设备名：{name:?}");
                if let Some(name) = &name {
                    assert!(!name.is_empty(), "读到的设备名不该是空串");
                    assert!(!name.contains('\0'), "读到的设备名不该带 NUL");
                    assert!(
                        name.len() <= 248,
                        "GATT Device Name 上限 248 字节，实际 {}",
                        name.len()
                    );
                }

                // (a) 原始帧上复查三条生产级门控：回声门控 / LEN / 校验和。
                assert!(
                    frame.starts_with(&RESPONSE_PREFIX),
                    "应答必须以 09 FF 开头（回声门控），实际 {}",
                    hex(&frame)
                );
                assert_eq!(frame[KEY_OFFSET], KEY_STATUS, "key 必须是 0x01");
                assert_eq!(
                    frame[IDX_OFFSET] & IDX_MASK,
                    IDX_BATTERY,
                    "idx 抹掉写标志后必须是 0x01"
                );
                assert_eq!(
                    usize::from(frame[LEN_OFFSET]),
                    frame.len(),
                    "LEN 必须等于整帧长度"
                );
                let (body, trailing) = frame.split_at(frame.len() - 1);
                assert_eq!(
                    checksum(body),
                    trailing[0],
                    "校验和必须等于前方所有字节之和 & 0xFF"
                );

                // 百分比槽位：名字 / 解码值 / 在 `data` 里的偏移。
                let percentages = [
                    ("left", battery.left, BATTERY_LEFT),
                    ("right", battery.right, BATTERY_RIGHT),
                    ("case", battery.case, BATTERY_CASE),
                ];

                // (b) 至少一只耳机报出真实电量。另一只不可用是正常状态，不算失败。
                assert!(
                    [battery.left, battery.right]
                        .into_iter()
                        .flatten()
                        .any(|percent| (1..=100).contains(&percent)),
                    "至少一只耳机应当报出 1..=100 的电量，实际 left={:?} right={:?}",
                    battery.left,
                    battery.right
                );

                // (c) 所有出现的百分比都必须在合法范围（充电仓也算）。
                for (name, slot, _) in percentages {
                    if let Some(percent) = slot {
                        assert!(
                            percent <= 100,
                            "{name} 电量应当落在 0..=100，实际 {percent}"
                        );
                    }
                }

                // (d) `0xFF -> None` 必须真的成立：拿原始帧的字节对，不看解码结论。
                for (name, slot, offset) in percentages {
                    if slot.is_none() {
                        assert_eq!(
                            frame[DATA_OFFSET + offset],
                            UNAVAILABLE,
                            "{name} 为 None 时，原始帧里对应的数据字节必须是 0xFF"
                        );
                    }
                }

                // 充电位的 `None` 语义是「既不是 0 也不是 1」（实测该侧不通信时为 0xFF），
                // 因此这里钉的是「Some <-> 0/1、None <-> 其它」，而不是 0xFF。
                for (name, slot, offset) in [
                    ("leftCharging", battery.left_charging, BATTERY_LEFT_CHARGING),
                    (
                        "rightCharging",
                        battery.right_charging,
                        BATTERY_RIGHT_CHARGING,
                    ),
                ] {
                    let raw = frame[DATA_OFFSET + offset];
                    match slot {
                        Some(true) => {
                            assert_eq!(raw, 0x01, "{name} 为 true 时原始字节必须是 0x01");
                        }
                        Some(false) => {
                            assert_eq!(raw, 0x00, "{name} 为 false 时原始字节必须是 0x00");
                        }
                        None => assert!(
                            !matches!(raw, 0x00 | 0x01),
                            "{name} 为 None 时原始字节必须既不是 0x00 也不是 0x01，实际 {raw:02X}"
                        ),
                    }
                }
            });
        });
        probe.join().expect("探针线程不应 panic");
    }

    #[test]
    fn later_sessions_discard_the_previous_one() {
        let state = BleState::new();
        let (first, _receiver) = state.begin_session(None).expect("首轮会话应当可以开始");
        state.end_session(first);

        let (second, _receiver) = state.begin_session(None).expect("第二轮会话应当可以开始");
        assert!(second > first);
        // 旧会话的收尾不该把新会话的通道清掉。
        state.end_session(first);
        assert!(
            state.enqueue(Outgoing::Stop).is_ok(),
            "新会话的通道仍然可用"
        );
        assert!(state
            .commit(first, |inner| inner.set_phase(BlePhase::Error))
            .is_none());
        assert!(state
            .commit(second, |inner| inner.set_phase(BlePhase::Ready))
            .is_some());
    }

    #[test]
    fn address_is_the_only_identity_signal() {
        // 地址是**唯一**的身份信号（实测耳机不改地址）。
        let target = target_address().expect("设备地址应当可解析");
        assert_eq!(target.to_string(), DEVICE_ADDRESS, "地址按大写冒号格式回显");

        // 厂商数据的尾巴曾经被当成第二判据（「payload 末 6 字节 = 自身地址」），
        // 现已被实测推翻：同一台耳机（地址始终如上）先后广播过下面两种 payload ——
        // 一次尾巴恰好等于自身地址，一次是稳定但不属于自己的 `a1020306cafd`。
        let tail_is_self = vec![
            0x00, 0x03, 0x00, 0x01, 0x00, 0x0b, 0x01, 0x64, 0x00, 0x12, 0x02, 0x03, 0x04, 0x7c,
            0x0f,
        ];
        let tail_is_other = vec![
            0x00, 0x03, 0x00, 0x01, 0x00, 0x0b, 0x01, 0x64, 0x00, 0xa1, 0x02, 0x03, 0x06, 0xca,
            0xfd,
        ];

        // 两种 payload 都属于**同一台**设备：身份只能靠地址，tail 变什么都不影响命中。
        for payload in [tail_is_self, tail_is_other] {
            let properties = PeripheralProperties {
                manufacturer_data: HashMap::from([(DEVICE_COMPANY_ID, payload.clone())]),
                ..PeripheralProperties::default()
            };
            assert_eq!(
                target.to_string(),
                DEVICE_ADDRESS,
                "身份判据里没有 payload 的位置"
            );
            // 厂商标识仍是**候选过滤**的正确依据：带 0x5452 就进列表（内容原样带出）。
            let candidate = candidate_of(target, &properties, &[]).expect("厂商标识命中即为候选");
            assert_eq!(candidate.manufacturer_payload, hex_lower(&payload));
        }

        // 公司标识不对 → 不进候选（与身份判据无关，这里只是过滤）。
        let other = PeripheralProperties {
            manufacturer_data: HashMap::from([(0x004C, vec![0x12, 0x02, 0x03, 0x04, 0x7C, 0x0F])]),
            ..PeripheralProperties::default()
        };
        assert!(!has_vendor_manufacturer_data(&other));
        assert!(candidate_of(target, &other, &[]).is_none());

        // 空 payload 不算命中；完全没有厂商数据也不算（名字缺失同样不能当命中）。
        let empty = PeripheralProperties {
            manufacturer_data: HashMap::from([(DEVICE_COMPANY_ID, Vec::new())]),
            ..PeripheralProperties::default()
        };
        assert!(!has_vendor_manufacturer_data(&empty));
        assert!(candidate_of(target, &empty, &[]).is_none());
        assert!(!has_vendor_manufacturer_data(
            &PeripheralProperties::default()
        ));
        assert!(candidate_of(target, &PeripheralProperties::default(), &[]).is_none());

        // 只要带公司标识就够（哪怕 payload 只有 1 字节）—— 尾巴不再是判据，
        // 因此「长度不够」不再能筛掉候选。
        let short = PeripheralProperties {
            manufacturer_data: HashMap::from([(DEVICE_COMPANY_ID, vec![0x00])]),
            ..PeripheralProperties::default()
        };
        assert!(has_vendor_manufacturer_data(&short));
        assert!(candidate_of(target, &short, &[]).is_some());
    }

    #[test]
    fn advertised_name_is_optional_and_never_required() {
        // 耳机不广播名字：`None` 是正常现象，不能当错误。
        let properties = PeripheralProperties {
            address: target_address().expect("设备地址应当可解析"),
            ..PeripheralProperties::default()
        };
        assert_eq!(properties.local_name, None);
        assert_eq!(advertised_name(&properties), None);

        let properties = PeripheralProperties {
            local_name: Some("StarRing Ultra_BLE".into()),
            ..PeripheralProperties::default()
        };
        assert_eq!(
            advertised_name(&properties),
            Some("StarRing Ultra_BLE".to_string())
        );

        // 只有 advertisement_name 时也要能用。
        let properties = PeripheralProperties {
            advertisement_name: Some("LE-audio".into()),
            ..PeripheralProperties::default()
        };
        assert_eq!(advertised_name(&properties), Some("LE-audio".to_string()));
    }

    /// 造一条设备记录（只关心自适应降噪这个本地记忆项时用）。
    fn device_entry(address: &str, adaptive_anc: bool) -> BleDeviceEntry {
        BleDeviceEntry {
            address: address.to_string(),
            name: Some("StarRing Ultra_BLE".into()),
            last_connected_at: Some(7),
            adaptive_anc,
        }
    }

    #[test]
    fn adaptive_anc_follows_local_memory() {
        // 设备在 `06/02` 里永远只报「固定降噪」（实测，见 `adaptive_anc_request`），
        // 而写请求后面都会补一次读回（`write_readback`）—— 所以只要 `apply` 敢把
        // 解出来的 `false` 写进快照，用户每点一次开关就会被读回打回关。
        // 本用例就是钉住这条：真机 payload（d1 = 01）走一遍 `apply`，本地记忆必须原样不动。
        let mut inner = Inner {
            address: Some(DEVICE_ADDRESS.to_string()),
            devices: vec![device_entry(DEVICE_ADDRESS, true)],
            ..Inner::idle()
        };
        assert_eq!(inner.remembered_adaptive_anc(), Some(true));

        // 真机抓到的 `06/02` 应答：`data = 00 01 00 00 00`（固定降噪，不是自适应）。
        assert!(inner.apply(&parse(&NOISE_ANC_RESPONSE)));
        assert_eq!(
            inner.noise_mode,
            Some(NoiseMode::Anc),
            "噪声模式照常并入快照"
        );
        assert_eq!(
            inner.remembered_adaptive_anc(),
            Some(true),
            "读回 d1 = 01 不得把本地记忆的自适应开关打回 false"
        );

        // 反向也一样：本地记忆为 false 时，设备推来的 `d1 = 02` 也不会把它改成 true
        // （设备报文对这一个字段没有发言权，口径只有一处）。
        let mut inner = Inner {
            address: Some(DEVICE_ADDRESS.to_string()),
            devices: vec![device_entry(DEVICE_ADDRESS, false)],
            ..Inner::idle()
        };
        let adaptive_push = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x02, 0x0E, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00,
        ]);
        assert!(inner.apply(&parse(&adaptive_push)));
        assert_eq!(inner.noise_mode, Some(NoiseMode::Anc));
        assert_eq!(inner.remembered_adaptive_anc(), Some(false));
    }

    #[test]
    fn snapshot_reports_the_remembered_value_per_device() {
        let other = "AA:BB:CC:DD:EE:FF";
        let devices = vec![
            device_entry(DEVICE_ADDRESS, true),
            device_entry(other, false),
        ];

        // 连接中的设备优先：就算目标指向另一台，显示的也是「现在这台」的记忆值。
        let connected = Inner {
            address: Some(DEVICE_ADDRESS.to_string()),
            active_address: Some(other.to_string()),
            devices: devices.clone(),
            ..Inner::idle()
        };
        assert_eq!(connected.remembered_adaptive_anc(), Some(true));

        // 还没连上（只切了目标）：用目标那台的记忆值 —— 换耳机时各看各的。
        let target_only = Inner {
            address: None,
            active_address: Some(other.to_string()),
            devices: devices.clone(),
            ..Inner::idle()
        };
        assert_eq!(target_only.remembered_adaptive_anc(), Some(false));

        // 地址大小写不同也是同一台设备。
        let lowercase = Inner {
            address: Some("aa:bb:cc:dd:ee:ff".into()),
            devices: devices.clone(),
            ..Inner::idle()
        };
        assert_eq!(lowercase.remembered_adaptive_anc(), Some(false));

        // 有当前设备但配置里还没有它的条目 → `false`（等于出厂默认）。
        let unknown_device = Inner {
            address: Some(DEVICE_ADDRESS.to_string()),
            devices: Vec::new(),
            ..Inner::idle()
        };
        assert_eq!(unknown_device.remembered_adaptive_anc(), Some(false));

        // 连目标都没有（什么都没连过）→ 未知。
        let nothing = Inner {
            devices,
            ..Inner::idle()
        };
        assert_eq!(nothing.remembered_adaptive_anc(), None);
        assert_eq!(dto(&nothing).adaptive_anc, None);
        // 快照字段确实取自这条派生路径（而不是某个设备报文写进去的字段）。
        assert_eq!(dto(&connected).adaptive_anc, Some(true));
        assert_eq!(dto(&target_only).adaptive_anc, Some(false));
    }

    #[test]
    fn toggling_adaptive_anc_is_persisted_per_device() {
        let other = "AA:BB:CC:DD:EE:FF";
        let mut config = AppConfig::default();

        // 没有条目时新建，且只写这一项（名字 / 最近连接时间留给别的路径）。
        remember_adaptive_anc(&mut config, DEVICE_ADDRESS, true);
        assert_eq!(config.ble_devices.len(), 1);
        assert!(config.ble_devices[0].adaptive_anc);
        assert_eq!(config.ble_devices[0].name, None);
        assert_eq!(config.ble_devices[0].last_connected_at, None);

        // 第二台各记各的；大小写不同视为同一台。
        remember_adaptive_anc(&mut config, other, true);
        remember_adaptive_anc(&mut config, "12:02:03:04:7c:0f", false);
        assert_eq!(config.ble_devices.len(), 2, "大小写不同不该新建条目");
        assert!(!config.ble_devices[0].adaptive_anc, "目标那台被写回 false");
        assert!(config.ble_devices[1].adaptive_anc);
        assert_eq!(config.ble_devices[0].address, DEVICE_ADDRESS);

        // 落盘 / 读回一轮：值确实在配置里（前端下次启动还能读到）。
        let json = serde_json::to_string(&config).expect("配置应当可序列化");
        let loaded: AppConfig = serde_json::from_str(&json).expect("配置应当可读回");
        assert!(loaded.ble_devices[1].adaptive_anc);
        assert!(!loaded.ble_devices[0].adaptive_anc);
    }

    #[test]
    fn current_device_prefers_the_connected_one() {
        let state = BleState::new();
        // 什么都没有 → 没有当前设备。
        assert_eq!(state.read().current_device(), None);

        state.apply_preferences(&AppConfig {
            ble_active_address: Some("AA:BB:CC:DD:EE:FF".into()),
            ..AppConfig::default()
        });
        // 只有目标：它就是当前设备（用户操作的是「正要去连的那台」）。
        assert_eq!(state.read().current_device(), Some("AA:BB:CC:DD:EE:FF"));

        // 连上之后以连接中的设备为准（`establish` 会填 `address`）。
        state.write().address = Some(DEVICE_ADDRESS.to_string());
        assert_eq!(state.read().current_device(), Some(DEVICE_ADDRESS));
    }

    #[test]
    fn reconnecting_discards_answer_pending_callers() {
        // 真机复现过：链路刚断、正处在退避重连里，用户点设置 → 命令层返回 Ok、前端不弹
        // 提示，命令却被丢在重连等待里（日志「链路重连中，忽略一条蓝牙命令」五连）。
        // 这里钉住修好后的行为：每条被丢弃的命令都要收到明确错误，谁也不能挂着。
        let (reply, mut receiver) = oneshot::channel();
        assert!(
            drain_while_reconnecting(Some(Outgoing::Write {
                frame: game_mode_request(true),
                reply,
            })),
            "丢弃一条写命令后，整轮会话应当继续重连"
        );
        let outcome = receiver.try_recv().expect("被丢弃的写命令必须收到回执");
        let message = outcome.expect_err("回执必须是错误，不能是 Ok");
        assert!(
            message.to_string().contains("重连"),
            "错误文案要说清原因：{message}"
        );

        // 请求同样要收到错误（否则调用方只能等到超时）。
        let (reply, mut receiver) = oneshot::channel();
        assert!(drain_while_reconnecting(Some(Outgoing::Request {
            key: KEY_NOISE,
            idx: IDX_ANC_GRADIENT,
            frame: anc_gradient_request(AncGradient::Deep),
            reply,
        })));
        assert!(receiver
            .try_recv()
            .expect("被丢弃的请求必须收到回执")
            .is_err());

        // 停止信号与通道关闭：整轮会话结束（不是「丢弃一条命令」）。
        assert!(!drain_while_reconnecting(Some(Outgoing::Stop)));
        assert!(!drain_while_reconnecting(None));
    }

    #[test]
    fn write_receipts_never_report_success_without_a_write() {
        // 回执只有四种可能，其中三种都必须变成错误 —— 「拿不准就别报成功」是这条修复的
        // 全部意义：命令层返回 Ok 而帧没写出去，前端就不会弹任何提示。
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("创建运行时失败");
        runtime.block_on(async {
            // 会话线程确认写出去了 → Ok。
            let (reply, receiver) = oneshot::channel();
            reply.send(Ok(())).expect("应当能回执");
            assert!(await_write_receipt(receiver, WRITE_ACK_TIMEOUT)
                .await
                .is_ok());

            // 会话线程报写失败 → 原样上抛，文案里带着原因。
            let (reply, receiver) = oneshot::channel();
            reply
                .send(Err(AppError::Other("写入耳机失败：链路断了".into())))
                .expect("应当能回执");
            let outcome = await_write_receipt(receiver, WRITE_ACK_TIMEOUT).await;
            let message = outcome.expect_err("写失败必须变成错误");
            assert!(message.to_string().contains("写入耳机失败"), "{message}");

            // 没有会话 / 会话线程直接退出 → 回执通道断开 → 错误（不是成功、也不是挂住）。
            let (reply, receiver) = oneshot::channel::<AppResult<()>>();
            drop(reply);
            let outcome = await_write_receipt(receiver, WRITE_ACK_TIMEOUT).await;
            assert!(outcome
                .expect_err("回执通道断开必须变成错误")
                .to_string()
                .contains("已断开"));

            // 会话线程迟迟不回执（例如正在扫描）→ 超时 → 明确报「可能未发送」。
            // 这里用一个极短的上限代替 `WRITE_ACK_TIMEOUT`，避免测试真的等 3 秒。
            let (reply, receiver) = oneshot::channel::<AppResult<()>>();
            let outcome = await_write_receipt(receiver, Duration::from_millis(10)).await;
            assert!(
                outcome
                    .expect_err("超时必须变成错误")
                    .to_string()
                    .contains("超时"),
                "超时文案要说明「可能未发送」"
            );
            // 超时后接收端已被丢弃：会话线程迟到的回执无人接收 —— 调用方早已拿到错误，
            // 不会因为一帧迟到又被当成成功。
            assert!(reply.send(Ok(())).is_err());
        });
    }

    #[test]
    fn writes_without_a_session_are_rejected_instead_of_queued() {
        // 没有会话时 `enqueue` 直接报错（`send_frames` 会把它上抛给前端）。
        let state = BleState::new();
        let (reply, mut receiver) = oneshot::channel();
        let outcome = state.enqueue(Outgoing::Write {
            frame: game_mode_request(true),
            reply,
        });
        assert!(outcome.is_err(), "没有会话时写命令必须是错误，不能是 Ok");
        // 回执通道随消息一起被丢弃：调用方拿到的是 `enqueue` 的错误，不会挂住。
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn log_lines_pair_the_write_with_the_reply() {
        // 两条 DEBUG 日志的形状必须一致（同样的 key/idx/data 顺序、原始字节、不解码），
        // 读日志的人才能把「我们要求什么」和「设备报回来什么」并排对照：
        //   写入设置 key=0x06 idx=0x87 data=00 01
        //   收到应答 key=0x06 idx=0x87 data=00 00
        let write = anc_gradient_request(AncGradient::Medium);
        assert_eq!(
            frame_summary(&write).as_deref(),
            Some("key=0x06 idx=0x87 data=00 01"),
            "写帧原样给出带写标志位的 idx"
        );

        // 设备对同一次写的应答（`idx` 不带标志位）—— 两行只差 idx 的最高位，肉眼可比。
        let reply = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x07, 0x0B, 0x00, 0x00, 0x00,
        ]);
        assert_eq!(
            frame_summary(&reply).as_deref(),
            Some("key=0x06 idx=0x07 data=00 00"),
            "设备报的是 0（没接受），日志只给事实、不下结论"
        );

        // 单字节 ACK 也照样占一行：这正是「设备回了、但没带新值」的证据。
        let ack = resign(vec![
            0x09, 0xFF, 0x00, 0x00, 0x01, 0x06, 0x87, 0x0A, 0x00, 0x00,
        ]);
        assert_eq!(
            frame_summary(&ack).as_deref(),
            Some("key=0x06 idx=0x87 data=00")
        );

        // 电量轮询的应答（45 秒一行，不是每次都刷）。
        assert_eq!(
            frame_summary(&BATTERY_FRAME_LEFT_CHARGING).as_deref(),
            Some("key=0x01 idx=0x01 data=00 01 01 64 5D 01 00 4A 00")
        );

        // 太短的帧不给日志（也不 panic）。
        assert_eq!(frame_summary(&[]), None);
        assert_eq!(frame_summary(&[0x08, 0xEE]), None);
        assert_eq!(frame_summary(&REQUEST_PREFIX), None);
    }

    #[test]
    fn pending_requests_expire() {
        let now = tokio::time::Instant::now();
        let (first, mut first_rx) = oneshot::channel();
        let (second, _second_rx) = oneshot::channel();
        let mut pending = vec![
            Pending {
                key: KEY_STATUS,
                idx: IDX_BATTERY,
                deadline: now,
                reply: first,
            },
            Pending {
                key: KEY_EQ,
                idx: IDX_EQ,
                deadline: now + RESPONSE_TIMEOUT,
                reply: second,
            },
        ];

        expire_pending(&mut pending, now);
        assert_eq!(pending.len(), 1, "只有到期的那个被回收");
        assert_eq!(pending[0].key, KEY_EQ);
        // 到期的请求必须收到明确错误，命令层的 await 才不会永久挂起。
        let outcome = first_rx.try_recv().expect("应当已经回了错误");
        assert!(outcome.is_err());
    }

    #[test]
    fn candidate_field_names_match_the_frozen_contract() {
        // `scan_ble_devices` 的返回项：前端按这些键名读，改名即破坏契约。
        let candidate = BleCandidate {
            address: DEVICE_ADDRESS.to_string(),
            name: Some("StarRing Ultra_BLE".into()),
            rssi: -42,
            manufacturer_payload: "00030001000b016400120203047c0f".into(),
            known: true,
        };
        let value = serde_json::to_value(&candidate).expect("候选项应当可序列化");
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("候选项应当是对象")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["address", "known", "manufacturerPayload", "name", "rssi"]
        );
        assert_eq!(value["rssi"], -42);
        assert_eq!(value["name"], "StarRing Ultra_BLE");
    }

    #[test]
    fn device_name_decoding_strips_trailing_nuls_and_whitespace() {
        let name = decode_device_name(b"StarRing Ultra_BLE");
        assert_eq!(name.as_deref(), Some("StarRing Ultra_BLE"));
        let name = decode_device_name(b"StarRing Ultra_BLE\0");
        assert_eq!(name.as_deref(), Some("StarRing Ultra_BLE"));
        let name = decode_device_name(b"StarRing Ultra_BLE\0\0 \r\n");
        assert_eq!(name.as_deref(), Some("StarRing Ultra_BLE"));

        // 空 / 只有 NUL 与空白 = 没读到：调用方必须保持原样，而不是把名字清空。
        assert_eq!(decode_device_name(b""), None);
        assert_eq!(decode_device_name(b"\0"), None);
        assert_eq!(decode_device_name(b"   \0 "), None);

        // UTF-8 名字照常解码；非法字节降级成替换字符，绝不让会话失败。
        let name = decode_device_name("耳机\0".as_bytes());
        assert_eq!(name.as_deref(), Some("耳机"));
        let name = decode_device_name(&[0xFF, 0xFE]);
        assert_eq!(name.as_deref(), Some("\u{FFFD}\u{FFFD}"));
    }

    #[test]
    fn candidates_only_carry_the_earbud_manufacturer_id() {
        let address = target_address().expect("设备地址应当可解析");
        // 真机广播的厂商数据（见协议笔记 §15.2）：前 9 字节固定前缀，末 6 字节是自身地址。
        let payload = vec![
            0x00, 0x03, 0x00, 0x01, 0x00, 0x0b, 0x01, 0x64, 0x00, 0x12, 0x02, 0x03, 0x04, 0x7c,
            0x0f,
        ];
        let properties = PeripheralProperties {
            manufacturer_data: HashMap::from([(DEVICE_COMPANY_ID, payload)]),
            rssi: Some(-35),
            ..PeripheralProperties::default()
        };

        let candidate = candidate_of(address, &properties, &[]).expect("厂商数据命中即视为候选");
        assert_eq!(candidate.address, DEVICE_ADDRESS);
        assert_eq!(candidate.rssi, -35);
        assert_eq!(candidate.name, None, "耳机不广播名字：缺失是正常现象");
        assert!(!candidate.known);
        // 厂商数据原样给出（小写十六进制、无分隔符），不补齐也不臆造。
        assert_eq!(
            candidate.manufacturer_payload,
            "00030001000b016400120203047c0f"
        );

        // 已知设备：`known` 为真，且广播没名字时回落到缓存名。
        let cached = vec![BleDeviceEntry {
            address: DEVICE_ADDRESS.to_string(),
            name: Some("StarRing Ultra_BLE".into()),
            last_connected_at: Some(7),
            adaptive_anc: false,
        }];
        let candidate = candidate_of(address, &properties, &cached).expect("应当仍是候选");
        assert!(candidate.known);
        assert_eq!(candidate.name.as_deref(), Some("StarRing Ultra_BLE"));

        // 广播名优先于缓存名（其它型号的耳机是带名字的）。
        let named = PeripheralProperties {
            local_name: Some("LE-audio".into()),
            ..properties.clone()
        };
        let candidate = candidate_of(address, &named, &cached).expect("应当仍是候选");
        assert_eq!(candidate.name.as_deref(), Some("LE-audio"));

        // 平台没报告信号强度时为 0，而不是编一个值出来。
        let unnamed = PeripheralProperties {
            manufacturer_data: HashMap::from([(DEVICE_COMPANY_ID, vec![0x00])]),
            rssi: None,
            ..PeripheralProperties::default()
        };
        let candidate = candidate_of(address, &unnamed, &[]).expect("应当仍是候选");
        assert_eq!(candidate.rssi, 0);

        // 没有厂商数据 / 公司标识不对的设备一律不出现在候选里
        // （否则列表就是一整屏无法辨认的 MAC 地址）。
        assert!(candidate_of(address, &PeripheralProperties::default(), &[]).is_none());
        let other = PeripheralProperties {
            manufacturer_data: HashMap::from([(0x004C, vec![0x12, 0x02, 0x03, 0x04, 0x7C, 0x0F])]),
            ..PeripheralProperties::default()
        };
        assert!(candidate_of(address, &other, &[]).is_none());
    }

    #[test]
    fn watching_is_derived_from_the_watcher_and_the_session() {
        let mut inner = Inner {
            auto_connect: true,
            active_address: Some(DEVICE_ADDRESS.to_string()),
            watcher_running: true,
            ..Inner::idle()
        };
        assert!(
            inner.is_watching(),
            "自动连接 + 有目标 + 没有会话 = 正在监视"
        );

        // 会话在跑（搜索 / 连接中 / 已连接）时不是「在监视」：看门狗此刻不扫描。
        for phase in [BlePhase::Scanning, BlePhase::Connecting, BlePhase::Ready] {
            inner.phase = phase;
            assert!(!inner.is_watching(), "{phase:?} 阶段不该报 watching");
        }
        // `error` 时没有会话线程在跑，所以回到「在监视」。
        inner.phase = BlePhase::Error;
        assert!(inner.is_watching());

        // 用户主动断开（抑制）：不再自动重连。
        inner.suppressed = true;
        assert!(!inner.is_watching());
        inner.suppressed = false;

        // 缺任何一条都不成立：开关关了 / 没有目标 / 没有线程在跑。
        inner.auto_connect = false;
        assert!(!inner.is_watching());
        inner.auto_connect = true;
        inner.active_address = None;
        assert!(!inner.is_watching());
        inner.active_address = Some(DEVICE_ADDRESS.to_string());
        inner.watcher_running = false;
        assert!(!inner.is_watching());
    }

    #[test]
    fn config_mirror_survives_session_resets() {
        let state = BleState::new();
        let config = AppConfig {
            ble_auto_connect: true,
            ble_active_address: Some(DEVICE_ADDRESS.to_string()),
            ble_devices: vec![BleDeviceEntry {
                address: DEVICE_ADDRESS.to_string(),
                name: Some("StarRing Ultra_BLE".into()),
                last_connected_at: Some(7),
                adaptive_anc: true,
            }],
            ..AppConfig::default()
        };
        assert!(state.apply_preferences(&config), "首次同步应当算「有变化」");
        assert!(!state.apply_preferences(&config), "同样的配置不该报变化");

        let snapshot = state.snapshot();
        assert_eq!(snapshot.active_address.as_deref(), Some(DEVICE_ADDRESS));
        assert!(snapshot.auto_connect);
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(
            snapshot.devices[0].name.as_deref(),
            Some("StarRing Ultra_BLE")
        );
        assert_eq!(snapshot.devices[0].last_connected_at, Some(7));
        // 还没连上，但目标已知：`adaptiveAnc` 取那台设备的本地记忆值。
        assert_eq!(snapshot.adaptive_anc, Some(true));

        // 起一轮会话再断开：已知列表 / 目标 / 开关都必须还在
        // （前端靠它们渲染列表，看门狗靠目标决定盯谁）。
        let (generation, _receiver) = state
            .begin_session(Some(DEVICE_ADDRESS.to_string()))
            .expect("首轮会话应当可以开始");
        state.request_cancel();
        state.end_session(generation);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, BlePhase::Idle);
        assert_eq!(snapshot.devices.len(), 1, "断开不该清掉已知设备");
        assert_eq!(snapshot.active_address.as_deref(), Some(DEVICE_ADDRESS));
        assert!(snapshot.auto_connect, "断开不该关掉自动连接开关");
    }

    #[test]
    fn manual_disconnect_suppresses_auto_connect_until_cleared() {
        let state = BleState::new();
        state.apply_preferences(&AppConfig {
            ble_auto_connect: true,
            ble_active_address: Some(DEVICE_ADDRESS.to_string()),
            ..AppConfig::default()
        });
        assert!(state.watch_wanted());

        // 看门狗起了一轮：快照立刻报「正在监视」。
        let generation = state.begin_watch().expect("首轮看门狗应当可以起");
        assert!(state.snapshot().watching);
        // 幂等：已在跑时不重复起线程。
        assert!(state.begin_watch().is_none());

        // 用户主动断开：抑制 + 停看门狗，两者都让 `watching` 落到 false。
        state.suppress();
        assert!(state.is_suppressed());
        assert!(!state.snapshot().watching, "抑制期间不得自动重连");
        assert!(!state.watch_wanted());
        assert!(state.stop_watch());
        assert!(!state.snapshot().watching);
        // 被停止的那一轮从此作废（在途线程据此退出）。
        assert!(!state.watch_is_current(generation));

        // 用户再点连接（或切一次自动连接开关）会清掉抑制，看门狗可以重新起。
        assert!(state.clear_suppressed());
        assert!(!state.is_suppressed());
        assert!(state.watch_wanted());
        assert!(state.begin_watch().is_some(), "抑制清掉后可以重新起看门狗");
    }

    #[test]
    fn session_target_gates_repeated_connect_requests() {
        let state = BleState::new();
        state.apply_preferences(&AppConfig {
            ble_active_address: Some(DEVICE_ADDRESS.to_string()),
            ..AppConfig::default()
        });

        // 没有会话时没有目标可谈。
        assert_eq!(state.session_target(), None);
        assert!(!state.owns_address(DEVICE_ADDRESS));

        let (_generation, _receiver) = state
            .begin_session(Some(DEVICE_ADDRESS.to_string()))
            .expect("首轮会话应当可以开始");
        assert_eq!(state.session_target().as_deref(), Some(DEVICE_ADDRESS));

        // 地址判定不分大小写：系统可能回报小写形式，`connect_earbuds` 的幂等比较
        // 依赖同一条规则（重复请求不该拆掉刚建好的链路）。
        assert!(state.owns_address("12:02:03:04:7c:0f"));
        assert!(state.owns_address(DEVICE_ADDRESS));
        assert!(!state.owns_address("AA:BB:CC:DD:EE:FF"));

        // 已连接的设备地址同样算「本会话拥有的地址」（连接成功后才会填上）。
        state.write().address = Some(DEVICE_ADDRESS.to_string());
        assert!(state.owns_address(DEVICE_ADDRESS));
    }

    #[test]
    fn upsert_device_dedupes_by_address() {
        let mut config = AppConfig::default();
        {
            let entry = upsert_device(&mut config, DEVICE_ADDRESS);
            entry.last_connected_at = Some(7);
            entry.name = Some("StarRing Ultra_BLE".into());
        }

        // 大小写不同、地址相同：不新增，改的是同一条，且保留首次写入的大小写。
        let entry = upsert_device(&mut config, "12:02:03:04:7c:0f");
        entry.last_connected_at = Some(9);
        assert_eq!(config.ble_devices.len(), 1);
        assert_eq!(config.ble_devices[0].last_connected_at, Some(9));
        assert_eq!(
            config.ble_devices[0].name.as_deref(),
            Some("StarRing Ultra_BLE")
        );
        assert_eq!(config.ble_devices[0].address, DEVICE_ADDRESS);

        // 新地址才追加（新条目的名字与时间都是空的，等真的连上再填）。
        let entry = upsert_device(&mut config, "AA:BB:CC:DD:EE:FF");
        assert_eq!(entry.name, None);
        assert_eq!(entry.last_connected_at, None);
        assert_eq!(config.ble_devices.len(), 2);
    }

    #[test]
    fn stale_whole_config_payloads_cannot_clobber_the_ble_keys() {
        // 主窗口与快捷小窗各持一份配置副本：`set_config` 回写的是**整份**配置，
        // 若它能改 BLE 键，那么「主窗口改主题」就会用过期副本覆盖掉另一窗口刚写下的
        // 活动设备 / 设备列表（lost update）。`preserve_config_keys` 就是那条防线。
        let authoritative = AppConfig {
            ble_auto_connect: true,
            ble_active_address: Some(DEVICE_ADDRESS.to_string()),
            ble_devices: vec![BleDeviceEntry {
                address: DEVICE_ADDRESS.to_string(),
                name: Some("StarRing Ultra_BLE".into()),
                last_connected_at: Some(7),
                adaptive_anc: true,
            }],
            ..AppConfig::default()
        };

        // 主窗口那份**过期**的副本：三个 BLE 键还是默认值，只是主题改成了深色。
        let mut incoming = AppConfig {
            theme: Theme::Dark,
            ..AppConfig::default()
        };
        preserve_config_keys(&mut incoming, &authoritative);

        assert_eq!(incoming.theme, Theme::Dark, "其它字段照旧生效");
        assert!(incoming.ble_auto_connect, "自动连接开关不能被整份回写改掉");
        assert_eq!(
            incoming.ble_active_address, authoritative.ble_active_address,
            "活动设备不能被整份回写改掉"
        );
        assert_eq!(
            incoming.ble_devices, authoritative.ble_devices,
            "已知设备列表不能被整份回写改掉"
        );

        // 反向同样成立：过期副本里塞了别的 BLE 值（含本地记忆的开关），也一律被换回权威值。
        let mut incoming = AppConfig {
            ble_auto_connect: false,
            ble_active_address: Some("AA:BB:CC:DD:EE:FF".into()),
            ble_devices: vec![BleDeviceEntry {
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: None,
                last_connected_at: None,
                adaptive_anc: false,
            }],
            ..AppConfig::default()
        };
        preserve_config_keys(&mut incoming, &authoritative);
        assert!(incoming.ble_auto_connect);
        assert_eq!(incoming.ble_active_address.as_deref(), Some(DEVICE_ADDRESS));
        assert_eq!(incoming.ble_devices.len(), 1);
        assert_eq!(incoming.ble_devices[0].address, DEVICE_ADDRESS);
        assert!(
            incoming.ble_devices[0].adaptive_anc,
            "本地记忆的自适应降噪也在被保护的三个键里（整份回写改不动它）"
        );
    }

    #[test]
    fn preserve_config_keys_masks_only_the_ble_keys() {
        // 反向守卫：别把 `preserve_config_keys` 写成「整份配置换回权威值」——
        // 主题 / 语言 / 快捷小窗 / 文件搜索都必须照 payload 生效，否则设置页就改不动了。
        let authoritative = AppConfig::default();
        let mut incoming = AppConfig {
            theme: Theme::Light,
            language: "en-US".into(),
            close_to_tray: false,
            quick_panel: true,
            file_search: true,
            file_search_roots: vec!["D:\\docs".into()],
            ..AppConfig::default()
        };
        preserve_config_keys(&mut incoming, &authoritative);

        assert_eq!(incoming.theme, Theme::Light);
        assert_eq!(incoming.language, "en-US");
        assert!(!incoming.close_to_tray);
        assert!(incoming.quick_panel);
        assert!(incoming.file_search);
        assert_eq!(incoming.file_search_roots, vec!["D:\\docs".to_string()]);
    }
}
