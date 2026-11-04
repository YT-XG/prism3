//! 鼠标电量：雷神 ML903 接收器的 HID feature report 私有协议。
//!
//! 协议来自官方驱动 `DriverCore.exe`（.NET / HidSharp）的 IL 反汇编 + 本机接收器实测，
//! 完整记录见 `docs/逆向/雷神ML903-鼠标电量HID协议.md`。
//!
//! 要点（实测，不是推测）：
//! - 设备是 2.4G 接收器 `VID_2022&PID_2869` 的 **MI_02** 接口；判据是
//!   `HidP_GetCaps().FeatureReportByteLength == 65`（同一复合设备的另外几个接口都是 0）。
//! - 帧长固定 65 字节，首字节是报告 ID（0），真正的字段从 `[3]` 开始：
//!   `[3] = 0x02` 固定、`[4]` 命令组、`[5]` 子索引、`[6]` 字段号（读 = 写值 | 0x80）。
//! - 电量：请求 `[4]=0x02 [5]=0x00 [6]=0x83`；应答 `[4]` 是负载长度、`[6]` 回显字段号，
//!   负载从 `[7]` 起：`[7]` 充电中（0/1）、`[8]` 百分比（0-100）。
//! - 有线模式下该字段恒为 0（官方驱动据此提示「请切换到 2.4G 模式」），所以
//!   `percent == 0` 一律按「未上报」处理，不当作真的 0%。
//!
//! 采集是**按需**的（前端卡片可见时轮询），不起常驻线程：HID 打开 + 一次 feature
//! 往返是毫秒级，30s 一次足够，没有设备时也不用为它留一个后台线程。

use std::sync::RwLock;

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// 厂商 ID（接收器与有线共用）。
pub const VENDOR_ID: u16 = 0x2022;
/// 2.4G 接收器的 PID：`0x2869` 是本机的 ML903 Dock；`0x2865` 是机型表里的「通配 Dongle」。
pub const WIRELESS_PIDS: [u16; 2] = [0x2869, 0x2865];
/// 有线模式 PID（该模式下电量字段恒为 0）。
pub const WIRED_PID: u16 = 0x2867;

/// 帧长：含首字节报告 ID，固定 65。
pub const FRAME_LEN: usize = 65;
/// 报告 ID（首字节）。
const REPORT_ID: u8 = 0x00;
/// 固定标记字节，请求与应答都必须是它。
const MARKER_INDEX: usize = 3;
const MARKER: u8 = 0x02;
/// 命令组 / 子索引 / 字段号的下标。
const GROUP_INDEX: usize = 4;
const SUB_INDEX: usize = 5;
const FIELD_INDEX: usize = 6;
/// 状态组的命令组号。
const GROUP_STATUS: u8 = 0x02;
/// 电量字段号（读操作 = `0x03 | 0x80`；写侧是 `0x03`，本模块只用读）。
const FIELD_BATTERY: u8 = 0x83;
/// 应答里「负载长度」与「负载起始」的下标。
const LEN_INDEX: usize = 4;
const PAYLOAD_INDEX: usize = 7;
/// 发完请求到读应答之间的等待（官方驱动是 `SpinWait.SpinUntil(…, 30)` 毫秒）。
const SETTLE: std::time::Duration = std::time::Duration::from_millis(30);

/// 一次读数的结果。`percent == None` 表示「没读到有效电量」（无设备 / 有线 / 设备没上报）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseBattery {
    /// 是否找到并成功打开了电量接口。
    pub connected: bool,
    /// 是否走 2.4G 接收器（有线模式下电量不可用）。
    pub wireless: bool,
    /// 电量百分比；`None` = 未上报。
    pub percent: Option<u8>,
    /// 是否正在充电。
    pub charging: bool,
}

impl MouseBattery {
    /// 「没插接收器」的默认值。
    pub const DISCONNECTED: Self = Self {
        connected: false,
        wireless: false,
        percent: None,
        charging: false,
    };
}

/// Tauri 托管的最近一次读数（多窗口共享；事件不缓存，挂载时靠它兜底）。
pub struct MouseState {
    last: RwLock<MouseBattery>,
}

impl MouseState {
    pub fn new() -> Self {
        Self {
            last: RwLock::new(MouseBattery::DISCONNECTED),
        }
    }

    pub fn current(&self) -> MouseBattery {
        *self
            .last
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn store(&self, next: MouseBattery) -> MouseBattery {
        let mut guard = self
            .last
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = next;
        next
    }
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

/// 构造电量请求帧（65 字节）。
pub fn battery_request() -> [u8; FRAME_LEN] {
    let mut frame = [0u8; FRAME_LEN];
    frame[0] = REPORT_ID;
    frame[MARKER_INDEX] = MARKER;
    frame[GROUP_INDEX] = GROUP_STATUS;
    frame[SUB_INDEX] = 0x00;
    frame[FIELD_INDEX] = FIELD_BATTERY;
    frame
}

/// 解析电量应答。
///
/// 只认「字段号回显 = `0x83`、负载长度 ≥ 2」的帧。设备会把请求原样回显到 feature
/// 缓冲里，回显的 `[6]` 同样等于 `0x83`，因此还要靠 `[1]` 的低位（应答标记 `0xA1`）
/// 与负载长度一起挡：回显帧的 `[4]` 是请求里的命令组，不是长度。
pub fn parse_battery(reply: &[u8]) -> Option<(u8, bool)> {
    if reply.len() < FRAME_LEN {
        return None;
    }
    if reply[0] != REPORT_ID || reply[MARKER_INDEX] != MARKER {
        return None;
    }
    if reply[FIELD_INDEX] != FIELD_BATTERY {
        return None;
    }
    if reply[LEN_INDEX] < 2 {
        return None;
    }
    let payload = &reply[PAYLOAD_INDEX..];
    Some((payload[1], payload[0] == 0x01))
}

/// 读一次鼠标电量。
///
/// 找不到设备（或设备被占用 / 读失败）都只回「未连接」，不报错——这是一个展示型指标，
/// 不该因为鼠标拔掉就在前端弹错误。
pub fn read_battery() -> AppResult<MouseBattery> {
    read_battery_inner()
}

#[cfg(windows)]
fn read_battery_inner() -> AppResult<MouseBattery> {
    let Some((path, wireless)) = win::find_battery_interface() else {
        return Ok(MouseBattery::DISCONNECTED);
    };
    let Some((percent, charging)) = win::read_battery_frame(&path)? else {
        // 打开了接口但没拿到有效帧：仍算连上（有线模式下就是这个表现）。
        return Ok(MouseBattery {
            connected: true,
            wireless,
            percent: None,
            charging: false,
        });
    };
    Ok(MouseBattery {
        connected: true,
        wireless,
        // 厂商驱动把 0 当「没有电量信息」（有线模式），这里保持一致。
        percent: (percent > 0).then_some(percent),
        charging,
    })
}

#[cfg(not(windows))]
fn read_battery_inner() -> AppResult<MouseBattery> {
    Ok(MouseBattery::DISCONNECTED)
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA,
    };
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid,
        HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS,
        PHIDP_PREPARSED_DATA,
    };
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    /// GENERIC_READ / GENERIC_WRITE（windows 0.62 把它们放在 Foundation 的访问掩码里，
    /// 这里直接用数值，省掉一条容易随版本漂移的 import）。
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    /// 打开的设备句柄：离开作用域即关闭。
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 用给定访问权限打开设备路径。
    fn open(path: &str, access: u32) -> Option<Handle> {
        let name = wide(path);
        let handle = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .ok()?;
        if handle.is_invalid() {
            return None;
        }
        Some(Handle(handle))
    }

    /// 这个接口是不是我们要的「电量接口」：VID 匹配、且 feature report 长度恰为 65。
    ///
    /// 同一复合设备的其它接口（MI_00 鼠标、MI_01 各 collection）feature 长度都是 0，
    /// 所以长度就是最稳的判据——官方驱动也是按 `MaxFeatureReportLength == 65` 挑的。
    fn probe_interface(path: &str) -> Option<u16> {
        let handle = open(path, 0)?;
        unsafe {
            let mut attributes = HIDD_ATTRIBUTES {
                Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
                ..Default::default()
            };
            if !HidD_GetAttributes(handle.0, &mut attributes) {
                return None;
            }
            if attributes.VendorID != VENDOR_ID {
                return None;
            }
            let mut preparsed = PHIDP_PREPARSED_DATA::default();
            if !HidD_GetPreparsedData(handle.0, &mut preparsed) {
                return None;
            }
            let mut caps = HIDP_CAPS::default();
            let status = HidP_GetCaps(preparsed, &mut caps);
            let _ = HidD_FreePreparsedData(preparsed);
            if status.is_err() {
                return None;
            }
            if caps.FeatureReportByteLength as usize != FRAME_LEN {
                return None;
            }
            Some(attributes.ProductID)
        }
    }

    /// 找出电量接口的设备路径（无线优先）。返回 `(路径, 是否无线)`。
    pub(super) fn find_battery_interface() -> Option<(String, bool)> {
        let mut found: Option<(String, bool)> = None;
        unsafe {
            let guid = HidD_GetHidGuid();
            let set = SetupDiGetClassDevsW(
                Some(&guid),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok()?;
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            let mut index = 0u32;
            while SetupDiEnumDeviceInterfaces(set, None, &guid, index, &mut data).is_ok() {
                index += 1;
                let mut needed = 0u32;
                let _ =
                    SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut needed), None);
                if needed == 0 {
                    continue;
                }
                // `SP_DEVICE_INTERFACE_DETAIL_DATA_W` 是「u32 + 变长宽字符路径」，按 8 字节对齐分配，
                // 否则 `*mut u32` 写入 cbSize 会是未对齐访问。
                let mut buffer = vec![0u64; needed as usize / 8 + 2];
                let detail = buffer.as_mut_ptr() as *mut u32;
                *detail = std::mem::size_of::<u32>() as u32 + 4;
                let ok = SetupDiGetDeviceInterfaceDetailW(
                    set,
                    &data,
                    Some(detail as *mut _),
                    needed,
                    None,
                    None,
                )
                .is_ok();
                if !ok {
                    continue;
                }
                // 路径紧跟在 cbSize 之后（偏移 4）。
                let chars = (detail as *const u16).add(2);
                let mut len = 0usize;
                while *chars.add(len) != 0 {
                    len += 1;
                }
                if len == 0 {
                    continue;
                }
                let path = String::from_utf16_lossy(std::slice::from_raw_parts(chars, len));
                let Some(pid) = probe_interface(&path) else {
                    continue;
                };
                let wireless = WIRELESS_PIDS.contains(&pid);
                if !wireless && pid != WIRED_PID {
                    continue;
                }
                if wireless {
                    let _ = SetupDiDestroyDeviceInfoList(set);
                    return Some((path, true));
                }
                if found.is_none() {
                    found = Some((path, false));
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
        }
        found
    }

    /// 发一次电量请求并读回应答，返回 `(百分比, 是否充电)`。
    pub(super) fn read_battery_frame(path: &str) -> AppResult<Option<(u8, bool)>> {
        let handle = open(path, GENERIC_READ | GENERIC_WRITE)
            .ok_or_else(|| AppError::Other("鼠标接口被其它程序独占，读不到电量".to_string()))?;

        let request = battery_request();
        let sent = unsafe {
            HidD_SetFeature(
                handle.0,
                request.as_ptr() as *const core::ffi::c_void,
                FRAME_LEN as u32,
            )
        };
        if !sent {
            return Ok(None);
        }

        std::thread::sleep(SETTLE);

        // 读之前先把标记字节写回去（官方驱动同样在 GetFeature 前写 `buf[3] = 0x02`）。
        let mut reply = battery_request();
        let got = unsafe {
            HidD_GetFeature(
                handle.0,
                reply.as_mut_ptr() as *mut core::ffi::c_void,
                FRAME_LEN as u32,
            )
        };
        if !got {
            return Ok(None);
        }
        Ok(parse_battery(&reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 ML903 接收器实测应答（`02/00/83` 请求）：`[7]=0x00` 未充电、`[8]=0x41`(65%)。
    const LIVE_REPLY: [u8; FRAME_LEN] = {
        let mut frame = [0u8; FRAME_LEN];
        frame[0] = 0x00;
        frame[1] = 0xA1;
        frame[3] = 0x02;
        frame[4] = 0x02;
        frame[6] = 0x83;
        frame[7] = 0x00;
        frame[8] = 0x41;
        frame
    };

    #[test]
    fn request_frame_matches_the_vendor_bytes() {
        let frame = battery_request();
        assert_eq!(frame.len(), 65);
        assert_eq!(frame[0], 0x00, "报告 ID 必须是 0");
        assert_eq!(frame[3], 0x02, "固定标记");
        assert_eq!(frame[4], 0x02, "状态组");
        assert_eq!(frame[5], 0x00);
        assert_eq!(frame[6], 0x83, "电量字段（读）");
        assert!(frame[7..].iter().all(|byte| *byte == 0), "其余补零");
    }

    #[test]
    fn parses_the_live_reply() {
        assert_eq!(parse_battery(&LIVE_REPLY), Some((0x41, false)));
    }

    #[test]
    fn parses_charging_flag() {
        let mut frame = LIVE_REPLY;
        frame[7] = 0x01;
        assert_eq!(parse_battery(&frame), Some((0x41, true)));
        // 不是 1 的任何值都按「未充电」处理（厂商只判 == 1）。
        frame[7] = 0x02;
        assert_eq!(parse_battery(&frame), Some((0x41, false)));
    }

    #[test]
    fn rejects_echo_and_short_frames() {
        // 请求原样回显：`[4]` 是命令组 0x02，但负载为空 —— 不能用它当读数。
        let mut echo = battery_request();
        echo[4] = 0x00;
        assert_eq!(parse_battery(&echo), None);

        // 只有 64 字节的帧不作数。
        assert_eq!(parse_battery(&LIVE_REPLY[..64]), None);

        // 字段号不对（别的命令的应答）。
        let mut other = LIVE_REPLY;
        other[6] = 0x81;
        assert_eq!(parse_battery(&other), None);

        // 标记字节不对。
        let mut bad_marker = LIVE_REPLY;
        bad_marker[3] = 0x00;
        assert_eq!(parse_battery(&bad_marker), None);
    }

    #[test]
    fn zero_percent_means_not_reported() {
        // 有线模式下设备回 0：官方驱动据此提示切 2.4G，这里同样按「未上报」。
        let mut frame = LIVE_REPLY;
        frame[8] = 0x00;
        let (percent, charging) = parse_battery(&frame).expect("帧仍然有效");
        assert_eq!(percent, 0);
        assert!(!charging);
    }

    /// 真机探针：插着 ML903 接收器时把整条链路跑一遍（枚举 → 开设备 → 写 → 读 → 解析）。
    ///
    /// 没插接收器时它会失败，因此默认 `#[ignore]`，手动跑：
    /// `cargo test -- --ignored reads_the_mouse_battery_from_the_receiver`
    ///
    /// **鼠标休眠时接收器会回 `0`**（实测同一台设备：刚操作过时 `[8]=0x41`，
    /// 静置十分钟后变 `0`，且应答 `[1]` 由 `0xA1` 变 `0xA4`），所以这里不把
    /// 「有百分比」当硬断言，只要求「能读到帧、编号合法」。
    #[test]
    #[ignore = "需要插着 ML903 接收器（真机探针）"]
    fn reads_the_mouse_battery_from_the_receiver() {
        let battery = read_battery().expect("读取不应报错");
        assert!(
            battery.connected,
            "应能找到 feature report 长度 65 的电量接口"
        );
        assert!(battery.wireless, "本机接的是 2.4G 接收器");
        if let Some(percent) = battery.percent {
            assert!(percent <= 100, "百分比不该超过 100，实际 {percent}");
            println!("ML903 电量 {percent}%（充电中 = {}）", battery.charging);
        } else {
            println!("ML903 已连上，但这次没有电量（鼠标休眠 / 有线模式，两者都是 0 哨兵）");
        }
    }
}
