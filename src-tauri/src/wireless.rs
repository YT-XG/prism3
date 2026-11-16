//! 多品牌无线外设电量（HID）：Logitech HID++ 2.0、CompX/ATK `1A 06`、标准 HID 电量字段。
//!
//! 与 `mouse.rs` 同一套口径：只读、按需采集、不弹窗、不抢输入。这里不引入 Python/侧车；
//! 协议来自公开实现（Solaar / HaloBattery）与厂商抓包记录，只在「能证明身份」的接口上发请求。
//!
//! 三个 provider：
//! - `logitech`：接收器/有线设备的 `FF00:0001`（短）/`FF00:0002`（长）集合，
//!   逐个槽位读 `0x1004` 统一电量（失败再退 `0x1000` 电量状态）。
//! - `compx`：精确 VID/PID + `FF1C:0092` 集合，输出 `04 … 1A 06`，回复第 8 字节为百分比。
//! - `standard`：HID 描述符里明确声明了 Battery Strength（`0x06:0x20`）或
//!   Battery Level（`0x85:0x64`）的 feature 字段，只 `GetFeature`，不写设备。
//!
//! **电量未知就返回 `None`**：休眠、有线、厂商驱动独占都可能是「没有读数」，
//! 绝不把 0 当成 0% 展示。

use std::sync::RwLock;

use serde::Serialize;

use crate::error::AppResult;

/// 一台无线外设的读数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WirelessBattery {
    /// 稳定标识（同一接口路径在一次枚举内唯一）。
    pub id: String,
    /// 展示名：设备自报名字优先，其次产品串，最后是型号表里的名字。
    pub name: String,
    /// `mouse` / `keyboard` / `headset` / `other`。
    pub kind: String,
    /// 百分比；`None` = 未上报（休眠 / 有线 / 设备不支持）。
    pub percent: Option<u8>,
    /// 充电状态；`None` = 该协议不提供，或本次没读到。
    pub charging: Option<bool>,
    /// 百分比是否由电压估算（Logitech 耳机 / 只报电压的设备），UI 显示「约」。
    pub approx: bool,
    /// 读数来源：`logitech` / `compx` / `standard`。
    pub source: String,
    /// 传输方式说明（「2.4G 接收器」/「USB HID」）。
    pub transport: String,
}

impl WirelessBattery {
    fn new(
        id: String,
        name: String,
        kind: &str,
        percent: Option<u8>,
        charging: Option<bool>,
        source: &str,
        transport: &str,
    ) -> Self {
        Self {
            id,
            name,
            kind: kind.to_string(),
            percent: percent.filter(|value| *value <= 100),
            charging,
            approx: false,
            source: source.to_string(),
            transport: transport.to_string(),
        }
    }
}

/// Tauri 托管的最近一次枚举结果（多窗口共享；事件不缓存，挂载时靠它兜底）。
pub struct WirelessState {
    last: RwLock<Vec<WirelessBattery>>,
}

impl WirelessState {
    pub fn new() -> Self {
        Self {
            last: RwLock::new(Vec::new()),
        }
    }

    pub fn current(&self) -> Vec<WirelessBattery> {
        self.last
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn store(&self, next: Vec<WirelessBattery>) -> Vec<WirelessBattery> {
        let mut guard = self
            .last
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = next.clone();
        next
    }
}

impl Default for WirelessState {
    fn default() -> Self {
        Self::new()
    }
}

/// 读一轮全部无线外设电量。
///
/// 没有设备时只做一次 HID 枚举（毫秒级）；某个 provider 失败不影响其它 provider。
pub fn read_all() -> AppResult<Vec<WirelessBattery>> {
    let interfaces = enumerate();
    let mut result = Vec::new();
    result.extend(logitech::read(&interfaces));
    result.extend(compx::read(&interfaces));
    result.extend(standard::read(&interfaces));

    result.sort_by(|left, right| {
        kind_rank(&left.kind)
            .cmp(&kind_rank(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(result)
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "mouse" => 0,
        "keyboard" => 1,
        "headset" => 2,
        _ => 3,
    }
}

/// 一个 HID 接口的最小画像（一次枚举拿全，供所有 provider 复用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    /// 顶层集合的 Usage Page / Usage（`HIDP_CAPS`）。
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
}

/// 枚举当前在场的 HID 接口。
///
/// 只以零访问权限打开设备读取属性与 caps，不写任何报告——键盘鼠标也在枚举范围内，
/// 所以这一步必须是无副作用的。
#[cfg(windows)]
pub(crate) fn enumerate() -> Vec<Interface> {
    win::enumerate()
}

#[cfg(not(windows))]
pub(crate) fn enumerate() -> Vec<Interface> {
    Vec::new()
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
        HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
        HidD_GetProductString, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS, PHIDP_PREPARSED_DATA,
    };
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    /// 只查属性用：不需要读写权限。
    const ACCESS_NONE: u32 = 0;

    pub(super) struct Handle(pub(super) HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub(super) fn wide(path: &str) -> Vec<u16> {
        path.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn open(path: &str, access: u32) -> Option<Handle> {
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

    /// 读产品串；取不到就回空串（很多接收器本来就不声明它）。
    pub(super) fn product_string(handle: HANDLE) -> String {
        let mut buffer = [0u16; 128];
        let ok = unsafe {
            HidD_GetProductString(
                handle,
                buffer.as_mut_ptr() as *mut core::ffi::c_void,
                std::mem::size_of_val(&buffer) as u32,
            )
        };
        if !ok {
            return String::new();
        }
        let len = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
        String::from_utf16_lossy(&buffer[..len]).trim().to_string()
    }

    fn probe(path: &str) -> Option<Interface> {
        let handle = open(path, ACCESS_NONE)?;
        unsafe {
            let mut attributes = HIDD_ATTRIBUTES {
                Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
                ..Default::default()
            };
            if !HidD_GetAttributes(handle.0, &mut attributes) {
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
            Some(Interface {
                path: path.to_string(),
                vid: attributes.VendorID,
                pid: attributes.ProductID,
                product: product_string(handle.0),
                usage_page: caps.UsagePage,
                usage: caps.Usage,
                input_len: caps.InputReportByteLength,
                output_len: caps.OutputReportByteLength,
                feature_len: caps.FeatureReportByteLength,
            })
        }
    }

    pub(super) fn enumerate() -> Vec<Interface> {
        let mut result = Vec::new();
        unsafe {
            let guid = HidD_GetHidGuid();
            let Ok(set) = SetupDiGetClassDevsW(
                Some(&guid),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            ) else {
                return result;
            };
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
                let chars = (detail as *const u16).add(2);
                let mut len = 0usize;
                while *chars.add(len) != 0 {
                    len += 1;
                }
                if len == 0 {
                    continue;
                }
                let path = String::from_utf16_lossy(std::slice::from_raw_parts(chars, len));
                if let Some(interface) = probe(&path) {
                    result.push(interface);
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
        }
        result
    }
}

/// 带超时的 HID 输出/输入通道（overlapped I/O）。
///
/// 厂商查询必须「发一帧、在窗口期内等回复」，同步 `ReadFile` 在没有回复时会永远挂住，
/// 所以这里统一走 `OVERLAPPED` + 事件等待；超时后 `CancelIoEx` 收尾，绝不留下未完成的 I/O。
#[cfg(windows)]
pub(crate) mod io {
    use super::win::{wide, Handle};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    /// GENERIC_READ / GENERIC_WRITE。
    pub(crate) const READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
    /// HRESULT_FROM_WIN32(ERROR_IO_PENDING) = 0x8007_03E5。
    const HR_PENDING: u32 = 0x8007_03E5;

    pub(crate) struct Channel {
        handle: Handle,
        event: HANDLE,
    }

    impl Drop for Channel {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.event);
            }
        }
    }

    impl Channel {
        /// 打开设备路径；带 `FILE_FLAG_OVERLAPPED`，否则下面的事件等待没有意义。
        pub(crate) fn open(path: &str) -> Option<Self> {
            let name = wide(path);
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    READ_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    None,
                )
            }
            .ok()?;
            if handle.is_invalid() {
                return None;
            }
            let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.ok()?;
            Some(Self {
                handle: Handle(handle),
                event,
            })
        }

        fn finish(
            &self,
            overlapped: &OVERLAPPED,
            started: windows::core::Result<()>,
            timeout_ms: u32,
        ) -> Option<u32> {
            if let Err(error) = started {
                // 非 pending 的失败（句柄被占、设备已拔）直接放弃。
                if error.code().0 as u32 != HR_PENDING {
                    return None;
                }
                let wait = unsafe { WaitForSingleObject(self.event, timeout_ms) };
                if wait != WAIT_OBJECT_0 {
                    unsafe {
                        let _ = CancelIoEx(self.handle.0, Some(overlapped));
                    }
                    let mut drained = 0u32;
                    unsafe {
                        let _ = GetOverlappedResult(self.handle.0, overlapped, &mut drained, true);
                    }
                    return None;
                }
            }
            let mut transferred = 0u32;
            unsafe { GetOverlappedResult(self.handle.0, overlapped, &mut transferred, false) }
                .ok()?;
            Some(transferred)
        }

        /// 写一帧（输出报告，含报告 ID）。
        pub(crate) fn write(&self, data: &[u8], timeout_ms: u32) -> bool {
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let mut written = 0u32;
            let started = unsafe {
                WriteFile(
                    self.handle.0,
                    Some(data),
                    Some(&mut written),
                    Some(&mut overlapped),
                )
            };
            matches!(
                self.finish(&overlapped, started, timeout_ms),
                Some(count) if count as usize == data.len()
            )
        }

        /// 读一帧（输入报告，含报告 ID）；超时回 `None`。
        pub(crate) fn read(&self, buffer: &mut [u8], timeout_ms: u32) -> Option<usize> {
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let mut read = 0u32;
            let started = unsafe {
                ReadFile(
                    self.handle.0,
                    Some(buffer),
                    Some(&mut read),
                    Some(&mut overlapped),
                )
            };
            self.finish(&overlapped, started, timeout_ms)
                .map(|count| count as usize)
        }
    }

    /// 同步句柄（只读 feature 用，不需要超时语义）。
    pub(super) fn open_sync(path: &str, access: u32) -> Option<Handle> {
        super::win::open(path, access)
    }
}

#[cfg(windows)]
pub(crate) use io::Channel;
#[cfg(not(windows))]
pub(crate) struct Channel;

#[cfg(not(windows))]
impl Channel {
    pub(crate) fn open(_path: &str) -> Option<Self> {
        None
    }
    pub(crate) fn write(&self, _data: &[u8], _timeout_ms: u32) -> bool {
        false
    }
    pub(crate) fn read(&self, _buffer: &mut [u8], _timeout_ms: u32) -> Option<usize> {
        None
    }
}

/// CompX/ATK 协议族（`FF1C:0092`，输出报告 `04 … 1A 06`）。
mod compx {
    use super::Channel;
    use super::*;

    /// 一个精确型号的查询定义。
    struct Model {
        vid: u16,
        pid: u16,
        name: &'static str,
        kind: &'static str,
        request: fn() -> [u8; 64],
        parse: fn(&[u8]) -> Option<u8>,
        timeout_ms: u32,
    }

    /// VGN 320F:5088 键盘接收器（VGN HUB 的只读电量查询）。
    pub(super) fn vgn_request() -> [u8; 64] {
        let mut frame = [0u8; 64];
        frame[..5].copy_from_slice(&[0x04, 0x00, 0x00, 0x1A, 0x06]);
        frame
    }

    /// 回复 64 字节、`[3]=0x1A [4]=0x06`，`[8]` 为百分比；`[7]=0xFF` 是休眠/离线。
    pub(super) fn vgn_parse(reply: &[u8]) -> Option<u8> {
        if reply.len() != 64 || reply[0] != 0x04 || reply[3] != 0x1A || reply[4] != 0x06 {
            return None;
        }
        if reply[7] == 0xFF || reply == vgn_request() {
            return None;
        }
        (reply[8] <= 100).then_some(reply[8])
    }

    /// HECATE G3M PRO（同一协议，输出帧多一个 `0x20` 子命令）。
    fn g3m_request() -> [u8; 64] {
        let mut frame = [0u8; 64];
        frame[..5].copy_from_slice(&[0x04, 0x20, 0x00, 0x1A, 0x06]);
        frame
    }

    fn g3m_parse(reply: &[u8]) -> Option<u8> {
        if reply.len() != 64 || reply[0] != 0x04 || reply[3] != 0x1A {
            return None;
        }
        if reply[7] == 0xFE || reply[7] == 0xFF || reply == g3m_request() {
            return None;
        }
        (reply[8] <= 100).then_some(reply[8])
    }

    const MODELS: &[Model] = &[
        Model {
            vid: 0x320F,
            pid: 0x5088,
            name: "VGN 2.4G 键盘",
            kind: "keyboard",
            request: vgn_request,
            parse: vgn_parse,
            // 该接收器约 1.03s 才回（VGN HUB 抓包与实机一致），给 2.5s 窗口。
            timeout_ms: 2500,
        },
        Model {
            vid: 0x320F,
            pid: 0x706E,
            name: "HECATE G3M PRO",
            kind: "mouse",
            request: g3m_request,
            parse: g3m_parse,
            timeout_ms: 1000,
        },
    ];

    pub(super) fn read(interfaces: &[Interface]) -> Vec<WirelessBattery> {
        let mut result = Vec::new();
        for model in MODELS {
            // 两个方向都必须是「含报告 ID 共 64 字节」的单一 ID 04 集合。
            let Some(interface) = interfaces.iter().find(|interface| {
                interface.vid == model.vid
                    && interface.pid == model.pid
                    && interface.usage_page == 0xFF1C
                    && interface.usage == 0x0092
                    && interface.output_len == 64
                    && interface.input_len == 64
            }) else {
                continue;
            };

            let Some(channel) = Channel::open(&interface.path) else {
                continue;
            };
            let request = (model.request)();
            if !channel.write(&request, 500) {
                continue;
            }

            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(model.timeout_ms as u64);
            let mut buffer = [0u8; 64];
            let mut percent = None;
            while let Some(count) = channel.read(&mut buffer, 60) {
                if channel_is_done(&buffer[..count], &mut percent, model) {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
            }
            let Some(percent) = percent else {
                continue;
            };

            let name = if interface.product.is_empty() {
                model.name.to_string()
            } else {
                interface.product.clone()
            };
            result.push(WirelessBattery::new(
                format!("compx:{}", interface.path),
                name,
                model.kind,
                Some(percent),
                None,
                "compx",
                "2.4G 接收器",
            ));
        }
        result
    }

    /// 读循环的一步：命中有效回复就写回 `percent` 并结束；命中「休眠/离线」也结束。
    fn channel_is_done(reply: &[u8], percent: &mut Option<u8>, model: &Model) -> bool {
        if let Some(value) = (model.parse)(reply) {
            *percent = Some(value);
            return true;
        }
        // VGN 的 `[7]=0xFF` 是「睡着了 / 不在位」，继续等只会白等满窗口。
        reply.len() == 64 && reply[0] == 0x04 && reply[3] == 0xFF
    }
}

/// Logitech HID++ 2.0：接收器按槽位、有线设备按 `0xFF`。
mod logitech {
    use super::Channel;
    use super::*;

    const VID: u16 = 0x046D;
    /// 短集合 `FF00:0001`（报告 ID 0x10）/ 长集合 `FF00:0002`（报告 ID 0x11）。
    const SHORT_USAGE: u16 = 0x0001;
    const LONG_USAGE: u16 = 0x0002;
    const SHORT_REPORT: u8 = 0x10;
    const LONG_REPORT: u8 = 0x11;
    /// HID++ 2.0 特性号。
    const F_NAME: u16 = 0x0005;
    const F_UNIFIED: u16 = 0x1004;
    const F_STATUS: u16 = 0x1000;
    const F_VOLTAGE: u16 = 0x1001;
    const F_ADC: u16 = 0x1F20;
    /// 单次请求等待窗口：休眠的无线电最多约 0.5s 才应答第一条。
    const TIMEOUT_MS: u32 = 700;
    /// 每个接收器最多试这些槽位（0xFF = 有线 / 耳机接收器）。
    const SLOTS: [u8; 7] = [1, 2, 3, 4, 5, 6, 0xFF];

    pub(super) enum Reply {
        Data(Vec<u8>),
        Error,
    }

    /// 一条 HID++ 通道（长集合，必要时配短集合接收错误应答）。
    struct Link {
        long: Channel,
        short: Option<Channel>,
        swid: u8,
    }

    impl Link {
        fn next_swid(&mut self) -> u8 {
            self.swid = self.swid % 0x0F + 1;
            self.swid
        }

        /// 发一条 2.0 请求并等回显同一 `slot/feature/function` 的应答。
        fn request(
            &mut self,
            slot: u8,
            feature: u8,
            function: u8,
            params: &[u8],
        ) -> Option<Vec<u8>> {
            let swid = self.next_swid();
            let function = (function << 4) | swid;
            let mut frame = [0u8; 20];
            frame[0] = LONG_REPORT;
            frame[1] = slot;
            frame[2] = feature;
            frame[3] = function;
            let count = params.len().min(16);
            frame[4..4 + count].copy_from_slice(&params[..count]);
            if !self.long.write(&frame, 500) {
                return None;
            }

            let deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(TIMEOUT_MS as u64);
            let mut long_buffer = [0u8; 21];
            let mut short_buffer = [0u8; 8];
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                let slice = remaining.as_millis().clamp(1, 40) as u32;

                if let Some(count) = self.long.read(&mut long_buffer, slice) {
                    if let Some(reply) = match_reply(&long_buffer[..count], slot, feature, function)
                    {
                        return match reply {
                            Reply::Data(data) => Some(data),
                            Reply::Error => None,
                        };
                    }
                }
                if let Some(short) = self.short.as_ref() {
                    if let Some(count) = short.read(&mut short_buffer, 5) {
                        if let Some(reply) =
                            match_reply(&short_buffer[..count], slot, feature, function)
                        {
                            return match reply {
                                Reply::Data(data) => Some(data),
                                Reply::Error => None,
                            };
                        }
                    }
                }
            }
        }

        fn feature_index(&mut self, slot: u8, feature: u16) -> Option<u8> {
            let reply = self.request(slot, 0, 0, &[(feature >> 8) as u8, feature as u8])?;
            let index = *reply.first()?;
            (index != 0).then_some(index)
        }

        fn device_name(&mut self, slot: u8, index: u8) -> String {
            let Some(reply) = self.request(slot, index, 0, &[]) else {
                return String::new();
            };
            let length = reply[0] as usize;
            if length == 0 || length > 64 {
                return String::new();
            }
            let mut raw = Vec::with_capacity(length);
            while raw.len() < length {
                let Some(chunk) = self.request(slot, index, 1, &[raw.len() as u8]) else {
                    break;
                };
                if chunk.is_empty() {
                    break;
                }
                let take = chunk.len().min(16).min(length - raw.len());
                raw.extend_from_slice(&chunk[..take]);
            }
            String::from_utf8_lossy(&raw).trim().to_string()
        }

        /// `0x0005 fn2` 的设备类型：0/2 键盘、3/4/5 鼠标。
        fn device_kind(&mut self, slot: u8, index: u8) -> Option<&'static str> {
            let reply = self.request(slot, index, 2, &[])?;
            match reply.first()? {
                0 | 2 => Some("keyboard"),
                3..=5 => Some("mouse"),
                _ => None,
            }
        }

        /// 读一台设备的电量：优先 `0x1004`、`0x1000`，最后用电压估算。
        ///
        /// 返回 `(百分比, 充电中, 是否估算)`；估算只用于「设备只报电压」的耳机/鼠标，
        /// UI 会显示「约」，不假装 1% 精度。
        fn battery(&mut self, slot: u8) -> Option<(u8, bool, bool)> {
            if let Some(index) = self.feature_index(slot, F_UNIFIED) {
                if let Some(reply) = self.request(slot, index, 1, &[]) {
                    if reply.len() >= 3 {
                        let percent = reply[0];
                        let flags = reply[1];
                        let status = reply[2];
                        let percent = if percent == 0 {
                            flags_to_percent(flags).unwrap_or(0)
                        } else {
                            percent
                        };
                        if percent > 0 && percent <= 100 {
                            return Some((percent, on_charger(status), false));
                        }
                    }
                }
            }
            if let Some(index) = self.feature_index(slot, F_STATUS) {
                if let Some(reply) = self.request(slot, index, 0, &[]) {
                    if reply.len() >= 3 {
                        let percent = reply[0];
                        let status = reply[2];
                        if percent > 0 && percent <= 100 {
                            return Some((percent, on_charger(status), false));
                        }
                    }
                }
            }
            if let Some(index) = self.feature_index(slot, F_VOLTAGE) {
                if let Some(reply) = self.request(slot, index, 0, &[]) {
                    if reply.len() >= 3 {
                        let mv = ((reply[0] as u16) << 8) | reply[1] as u16;
                        if mv >= 2500 {
                            return Some((voltage_to_percent(mv), reply[2] & 0x80 != 0, true));
                        }
                    }
                }
            }
            if let Some(index) = self.feature_index(slot, F_ADC) {
                if let Some(reply) = self.request(slot, index, 0, &[]) {
                    if reply.len() >= 3 {
                        let mv = ((reply[0] as u16) << 8) | reply[1] as u16;
                        // bit0 = 耳机在位，bit1 = 充电中。
                        if mv >= 2500 && reply[2] & 0x01 != 0 {
                            return Some((voltage_to_percent(mv), reply[2] & 0x02 != 0, true));
                        }
                    }
                }
            }
            None
        }
    }

    /// 按请求身份过滤一条输入报告；错误应答的字段顺序与数据应答不同。
    pub(super) fn match_reply(reply: &[u8], slot: u8, feature: u8, function: u8) -> Option<Reply> {
        if reply.len() < 4 || reply[1] != slot {
            return None;
        }
        // 只认 HID++ 的短/长报告号，避免把其它集合的输入当成应答。
        if reply[0] != LONG_REPORT && reply[0] != SHORT_REPORT {
            return None;
        }
        if reply[2] == feature && reply[3] == function {
            return Some(Reply::Data(reply[4..].to_vec()));
        }
        // 11 <slot> 8F <feature> <function|swid> <error> …
        if (reply[2] == 0x8F || reply[2] == 0xFF)
            && reply.len() >= 6
            && reply[3] == feature
            && reply[4] == function
        {
            return Some(Reply::Error);
        }
        None
    }

    /// 0x1004 的 level flags → 近似档位（Solaar 的口径）。
    pub(super) fn flags_to_percent(flags: u8) -> Option<u8> {
        match flags {
            8 => Some(90),
            4 => Some(50),
            2 => Some(20),
            1 => Some(5),
            _ => None,
        }
    }

    /// 0x1004/0x1000 的状态字节：0 放电，1 充电，2 将满，3 满，4 慢充。
    pub(super) fn on_charger(status: u8) -> bool {
        (1..=4).contains(&status)
    }

    /// Solaar 的 Li-ion 放电曲线（mV → %），只用于「设备只报电压」的情况。
    const VOLTAGE_CURVE: [(u16, u8); 13] = [
        (4186, 100),
        (4067, 90),
        (3989, 80),
        (3922, 70),
        (3859, 60),
        (3811, 50),
        (3778, 40),
        (3751, 30),
        (3717, 20),
        (3671, 10),
        (3646, 5),
        (3579, 2),
        (3500, 0),
    ];

    pub(super) fn voltage_to_percent(mv: u16) -> u8 {
        let (top_mv, top_percent) = VOLTAGE_CURVE[0];
        if mv >= top_mv {
            return top_percent;
        }
        for window in VOLTAGE_CURVE.windows(2) {
            let (hi_mv, hi_percent) = window[0];
            let (lo_mv, lo_percent) = window[1];
            if mv >= lo_mv {
                let span = hi_mv - lo_mv;
                let delta = (mv - lo_mv) as u32 * (hi_percent - lo_percent) as u32;
                return lo_percent + ((delta + span as u32 / 2) / span as u32) as u8;
            }
        }
        0
    }

    /// 接收器实例：路径的 `#` 第 3 段去掉最后一个 `&collection`。
    /// 同一接收器的短/长集合只差最后一段，必须合并成一条通道。
    pub(super) fn instance(path: &str) -> String {
        path.split('#')
            .nth(2)
            .map(|part| {
                let lower = part.to_lowercase();
                lower
                    .rsplit_once('&')
                    .map(|(head, _)| head.to_string())
                    .unwrap_or(lower)
            })
            .unwrap_or_else(|| path.to_lowercase())
    }

    pub(super) fn read(interfaces: &[Interface]) -> Vec<WirelessBattery> {
        // (实例, 长集合路径, 短集合路径)
        let mut groups: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        for interface in interfaces {
            if interface.vid != VID || interface.usage_page != 0xFF00 {
                continue;
            }
            if interface.usage != SHORT_USAGE && interface.usage != LONG_USAGE {
                continue;
            }
            let key = instance(&interface.path);
            let position = match groups.iter().position(|(existing, _, _)| *existing == key) {
                Some(position) => position,
                None => {
                    groups.push((key, None, None));
                    groups.len() - 1
                }
            };
            if interface.usage == LONG_USAGE {
                groups[position].1 = Some(interface.path.clone());
            } else {
                groups[position].2 = Some(interface.path.clone());
            }
        }

        let mut result = Vec::new();
        for (key, long_path, short_path) in groups {
            let Some(long_path) = long_path else {
                continue;
            };
            let Some(long) = Channel::open(&long_path) else {
                continue;
            };
            let mut link = Link {
                long,
                short: short_path.as_deref().and_then(Channel::open),
                swid: 0,
            };

            for slot in SLOTS {
                // ping（root 特性 fn1）：槽位里没有设备时会收到错误应答，跳过即可。
                if link.request(slot, 0, 1, &[]).is_none() {
                    continue;
                }

                let Some((percent, charging, approx)) = link.battery(slot) else {
                    continue;
                };

                let (name, mut kind) = match link.feature_index(slot, F_NAME) {
                    Some(index) => (
                        link.device_name(slot, index),
                        link.device_kind(slot, index).unwrap_or("other").to_string(),
                    ),
                    None => (String::new(), "other".to_string()),
                };
                if link.feature_index(slot, F_ADC).is_some() {
                    kind = "headset".to_string();
                }

                let mut entry = WirelessBattery::new(
                    format!("logitech:{key}:{slot:02X}"),
                    if name.is_empty() {
                        format!("Logitech 设备 {slot:02X}")
                    } else {
                        name
                    },
                    &kind,
                    Some(percent),
                    Some(charging),
                    "logitech",
                    if slot == 0xFF {
                        "USB 有线"
                    } else {
                        "2.4G 接收器"
                    },
                );
                entry.approx = approx;
                result.push(entry);
            }
        }
        result
    }
}

/// 标准 HID 电量字段：描述符明确声明的 Battery Strength / Battery Level。
mod standard {
    use super::*;

    /// 已被专用 provider 接管的厂商，避免同一台设备出现两条读数。
    const HANDLED_VIDS: [u16; 2] = [0x046D, 0x320F];

    pub(super) fn read(interfaces: &[Interface]) -> Vec<WirelessBattery> {
        let mut result = Vec::new();
        for interface in interfaces {
            if HANDLED_VIDS.contains(&interface.vid) || interface.feature_len < 2 {
                continue;
            }
            if let Some((percent, kind)) = read_interface(interface) {
                let name = if interface.product.is_empty() {
                    "无线 HID 设备".to_string()
                } else {
                    interface.product.clone()
                };
                result.push(WirelessBattery::new(
                    format!("standard:{}", interface.path),
                    name,
                    kind,
                    Some(percent),
                    None,
                    "standard",
                    "USB HID",
                ));
            }
        }
        result
    }

    #[cfg(windows)]
    fn read_interface(interface: &Interface) -> Option<(u8, &'static str)> {
        use windows::Win32::Devices::HumanInterfaceDevice::{
            HidD_FreePreparsedData, HidD_GetFeature, HidD_GetPreparsedData, HidP_Feature,
            HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps, HIDP_CAPS, HIDP_VALUE_CAPS,
            PHIDP_PREPARSED_DATA,
        };

        let handle = super::io::open_sync(&interface.path, 0x8000_0000)?;
        unsafe {
            let mut preparsed = PHIDP_PREPARSED_DATA::default();
            if !HidD_GetPreparsedData(handle.0, &mut preparsed) {
                return None;
            }
            let mut caps = HIDP_CAPS::default();
            if HidP_GetCaps(preparsed, &mut caps).is_err() || caps.NumberFeatureValueCaps == 0 {
                let _ = HidD_FreePreparsedData(preparsed);
                return None;
            }
            let total = caps.NumberFeatureValueCaps as usize;
            let mut value_caps = vec![HIDP_VALUE_CAPS::default(); total];
            let mut count = total as u16;
            if HidP_GetValueCaps(HidP_Feature, value_caps.as_mut_ptr(), &mut count, preparsed)
                .is_err()
            {
                let _ = HidD_FreePreparsedData(preparsed);
                return None;
            }
            value_caps.truncate(count as usize);

            let mut found = None;
            for caps in &value_caps {
                if let Some((page, usage)) = battery_usage(caps) {
                    found = Some((
                        caps.ReportID,
                        page,
                        usage,
                        caps.LinkCollection,
                        caps.LogicalMin,
                        caps.LogicalMax,
                    ));
                    break;
                }
            }
            let Some((report_id, page, usage, link, min, max)) = found else {
                let _ = HidD_FreePreparsedData(preparsed);
                return None;
            };

            let length = caps.FeatureReportByteLength as usize;
            if !(2..=4096).contains(&length) {
                let _ = HidD_FreePreparsedData(preparsed);
                return None;
            }
            let mut report = vec![0u8; length];
            report[0] = report_id;
            let got = HidD_GetFeature(
                handle.0,
                report.as_mut_ptr() as *mut core::ffi::c_void,
                length as u32,
            );
            let mut value = 0u32;
            let status = HidP_GetUsageValue(
                HidP_Feature,
                page,
                Some(link),
                usage,
                &mut value,
                preparsed,
                &report,
            );
            let _ = HidD_FreePreparsedData(preparsed);
            if !got || status.is_err() {
                return None;
            }
            let percent = percent(value, min, max)?;
            Some((percent, interface_kind(interface)))
        }
    }

    /// 声明的电量 usage：Battery Strength（`0x06:0x20`）或 Battery Level（`0x85:0x64`）。
    #[cfg(windows)]
    fn battery_usage(
        caps: &windows::Win32::Devices::HumanInterfaceDevice::HIDP_VALUE_CAPS,
    ) -> Option<(u16, u16)> {
        unsafe {
            if caps.IsRange {
                let min = caps.Anonymous.Range.UsageMin;
                let max = caps.Anonymous.Range.UsageMax;
                if caps.UsagePage == 0x06 && min <= 0x20 && 0x20 <= max {
                    Some((caps.UsagePage, 0x20))
                } else if caps.UsagePage == 0x85 && min <= 0x64 && 0x64 <= max {
                    Some((caps.UsagePage, 0x64))
                } else {
                    None
                }
            } else {
                let usage = caps.Anonymous.NotRange.Usage;
                matches!((caps.UsagePage, usage), (0x06, 0x20) | (0x85, 0x64))
                    .then_some((caps.UsagePage, usage))
            }
        }
    }

    #[cfg(windows)]
    fn percent(value: u32, min: i32, max: i32) -> Option<u8> {
        if min < 0 || max <= min || max > 65535 {
            return None;
        }
        let value = value as i32;
        if value < min || value > max {
            return None;
        }
        Some((((value - min) * 100 + (max - min) / 2) / (max - min)) as u8)
    }

    fn interface_kind(interface: &Interface) -> &'static str {
        match (interface.usage_page, interface.usage) {
            (1, 2) => "mouse",
            (1, 6) => "keyboard",
            _ => "other",
        }
    }

    #[cfg(not(windows))]
    fn read_interface(_interface: &Interface) -> Option<(u8, &'static str)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compx_keyboard_reply_parses() {
        let mut reply = [0u8; 64];
        reply[0] = 0x04;
        reply[3] = 0x1A;
        reply[4] = 0x06;
        reply[8] = 93;
        assert_eq!(compx::vgn_parse(&reply), Some(93));
    }

    #[test]
    fn compx_rejects_echo_sleep_and_out_of_range() {
        let echo = compx::vgn_request();
        assert_eq!(compx::vgn_parse(&echo), None);

        let mut asleep = echo;
        asleep[7] = 0xFF;
        asleep[8] = 55;
        assert_eq!(compx::vgn_parse(&asleep), None);

        let mut bad = echo;
        bad[8] = 101;
        assert_eq!(compx::vgn_parse(&bad), None);
    }

    #[test]
    fn logitech_battery_flags_map_to_percent() {
        assert_eq!(logitech::flags_to_percent(8), Some(90));
        assert_eq!(logitech::flags_to_percent(4), Some(50));
        assert_eq!(logitech::flags_to_percent(2), Some(20));
        assert_eq!(logitech::flags_to_percent(1), Some(5));
        assert_eq!(logitech::flags_to_percent(0), None);
    }

    #[test]
    fn logitech_status_maps_to_charging() {
        assert!(!logitech::on_charger(0));
        for status in 1..=4 {
            assert!(logitech::on_charger(status), "状态 {status} 应在充电");
        }
        assert!(!logitech::on_charger(5));
    }

    #[test]
    fn logitech_matches_data_and_error_replies() {
        // 11 slot feature (fn<<4|swid) <payload>
        let data = [0x11u8, 0x01, 0x08, 0x21, 0x64, 0x00, 0x00];
        assert!(matches!(
            logitech::match_reply(&data, 0x01, 0x08, 0x21),
            Some(logitech::Reply::Data(_))
        ));
        assert!(logitech::match_reply(&data, 0x02, 0x08, 0x21).is_none());
        assert!(logitech::match_reply(&data, 0x01, 0x09, 0x21).is_none());

        // 11 slot 8F feature (fn<<4|swid) error
        let error = [0x11u8, 0x01, 0x8F, 0x08, 0x21, 0x08, 0x00];
        assert!(matches!(
            logitech::match_reply(&error, 0x01, 0x08, 0x21),
            Some(logitech::Reply::Error)
        ));
    }

    #[test]
    fn logitech_voltage_curve_is_monotonic() {
        // 满电、中点与欠压三档；曲线只用于「只报电压」的设备。
        assert_eq!(logitech::voltage_to_percent(4200), 100);
        assert_eq!(logitech::voltage_to_percent(3811), 50);
        assert_eq!(logitech::voltage_to_percent(3400), 0);
        let mut last = 0;
        for mv in (3500..=4200).step_by(10) {
            let level = logitech::voltage_to_percent(mv);
            assert!(level >= last, "电压越高电量不该下降（{mv}mV）");
            last = level;
        }
    }

    #[test]
    fn logitech_groups_short_and_long_collections() {
        // 同一接收器的短/长集合只差最后的 collection 段，必须归到同一实例。
        let short = r"\\?\HID#VID_046D&PID_C52B&MI_02#7&1234abcd&0&0000#{4d1e55b2}";
        let long = r"\\?\HID#VID_046D&PID_C52B&MI_02#7&1234abcd&0&0001#{4d1e55b2}";
        assert_eq!(logitech::instance(short), logitech::instance(long));
    }

    #[test]
    fn wireless_battery_filters_impossible_percent() {
        let battery = WirelessBattery::new(
            "x".into(),
            "设备".into(),
            "mouse",
            Some(120),
            None,
            "compx",
            "2.4G 接收器",
        );
        assert_eq!(battery.percent, None);
    }
}
