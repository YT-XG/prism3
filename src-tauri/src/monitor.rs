use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{DiskKind, Disks, NetworkData, Networks, System};

#[cfg(windows)]
use serde::Deserialize;

/// 磁盘列表的最小刷新间隔，降低采集开销。
const DISK_REFRESH_SECS: u64 = 5;

/// 单个磁盘分区的用量信息。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    /// 卷标，可能为空。
    pub name: String,
    /// 挂载点，Windows 上形如 `C:`。
    pub mount_point: String,
    pub file_system: String,
    pub kind: String,
    pub total: u64,
    pub available: u64,
    pub is_removable: bool,
}

/// 单块显卡：型号 + 专用显存（字节，无法读取时为 0）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub memory: u64,
}

/// 单条内存：品牌 / 型号 / 容量 / 频率 / 世代。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryModuleInfo {
    pub manufacturer: String,
    pub part_number: String,
    pub capacity: u64,
    pub speed: u32,
    pub memory_type: String,
}

/// 一次轮询得到的动态快照：仅包含随时间变化的量，避免每秒重复回传静态信息。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    /// CPU 全局占用率（0-100）。首次采集无有效差值时为 `None`。
    pub cpu_usage: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    /// 网络下行速率（字节/秒）。
    pub net_rx_per_sec: f64,
    /// 网络上行速率（字节/秒）。
    pub net_tx_per_sec: f64,
    pub uptime_secs: u64,
    pub disks: Vec<DiskInfo>,
}

/// 不随时间变化的静态系统信息，构造时算一次；供前端缓存后与动态快照合并。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatics {
    pub hostname: String,
    pub os_name: String,
    pub os_long_name: String,
    pub os_version: String,
    pub ip: String,
    /// CPU 型号。
    pub cpu_brand: String,
    /// 主板型号（厂商 + 型号）。
    pub motherboard: String,
    /// 整机型号（厂商 + 型号）。
    pub system_model: String,
    /// 显卡列表（型号 + 显存）。
    pub gpus: Vec<GpuInfo>,
    /// 内存条列表。
    pub memory_modules: Vec<MemoryModuleInfo>,
}

struct MonitorInner {
    system: System,
    networks: Networks,
    disks: Disks,
    /// 是否已至少刷新过一次 CPU，用于跳过不准确的首帧。
    cpu_warmed: bool,
    last_net_refresh: Option<Instant>,
    last_disk_refresh: Option<Instant>,
}

/// 由 Tauri 托管的系统监控状态。
pub struct MonitorState {
    inner: Mutex<MonitorInner>,
    statics: SystemStatics,
}

impl MonitorState {
    pub fn new() -> Self {
        let networks = Networks::new_with_refreshed_list();

        // CPU 型号只需读一次；顺便把 cpu 列表刷出来，供后续占用率采样复用。
        let mut system = System::new();
        system.refresh_cpu_all();
        let cpu_brand = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .unwrap_or_default();

        let hardware = collect_hardware();

        let statics = SystemStatics {
            hostname: System::host_name().unwrap_or_else(|| "未知".to_string()),
            os_name: System::name().unwrap_or_else(|| "未知".to_string()),
            os_long_name: System::long_os_version().unwrap_or_else(|| "未知".to_string()),
            os_version: System::os_version().unwrap_or_else(|| "未知".to_string()),
            ip: primary_ipv4(&networks),
            cpu_brand,
            motherboard: hardware.motherboard,
            system_model: hardware.system_model,
            gpus: hardware.gpus,
            memory_modules: hardware.memory_modules,
        };

        Self {
            inner: Mutex::new(MonitorInner {
                system,
                networks,
                disks: Disks::new_with_refreshed_list(),
                cpu_warmed: false,
                last_net_refresh: None,
                last_disk_refresh: None,
            }),
            statics,
        }
    }

    /// 刷新并返回当前快照。按需调用（前端轮询），无常驻采样开销。
    pub fn snapshot(&self) -> SystemSnapshot {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        inner.system.refresh_cpu_usage();
        inner.system.refresh_memory();

        // 首次刷新只建立基线，差值不准确，先返回 None 让前端显示占位。
        let cpu_usage = if inner.cpu_warmed {
            Some(inner.system.global_cpu_usage())
        } else {
            None
        };
        inner.cpu_warmed = true;

        inner.networks.refresh(true);
        let now = Instant::now();
        let elapsed = inner
            .last_net_refresh
            .map(|last| now.duration_since(last).as_secs_f64())
            .unwrap_or(0.0);
        inner.last_net_refresh = Some(now);

        // `received()/transmitted()` 即「自上次 refresh 的增量」，除以实测耗时得到速率。
        let mut rx = 0u64;
        let mut tx = 0u64;
        for (name, data) in inner.networks.list() {
            if is_virtual_interface(name) || !has_routable_ipv4(data) {
                continue;
            }
            rx += data.received();
            tx += data.transmitted();
        }
        let (net_rx_per_sec, net_tx_per_sec) = if elapsed > 0.0 {
            (rx as f64 / elapsed, tx as f64 / elapsed)
        } else {
            (0.0, 0.0)
        };

        let disk_due = inner
            .last_disk_refresh
            .map(|last| now.duration_since(last) >= Duration::from_secs(DISK_REFRESH_SECS))
            .unwrap_or(true);
        if disk_due {
            inner.disks.refresh(true);
            inner.last_disk_refresh = Some(now);
        }

        let disks = inner
            .disks
            .list()
            .iter()
            .filter(|disk| disk.total_space() > 0)
            .map(|disk| DiskInfo {
                name: disk.name().to_string_lossy().into_owned(),
                mount_point: disk.mount_point().to_string_lossy().into_owned(),
                file_system: disk.file_system().to_string_lossy().into_owned(),
                kind: disk_kind_label(disk.kind()).to_string(),
                total: disk.total_space(),
                available: disk.available_space(),
                is_removable: disk.is_removable(),
            })
            .collect();

        SystemSnapshot {
            cpu_usage,
            mem_used: inner.system.used_memory(),
            mem_total: inner.system.total_memory(),
            net_rx_per_sec,
            net_tx_per_sec,
            uptime_secs: System::uptime(),
            disks,
        }
    }

    /// 返回静态系统信息（克隆一份，前端取一次后自行缓存）。
    pub fn statics(&self) -> SystemStatics {
        self.statics.clone()
    }
}

/// 硬件型号集合：进程内只采集一次。
struct HardwareInfo {
    motherboard: String,
    system_model: String,
    gpus: Vec<GpuInfo>,
    memory_modules: Vec<MemoryModuleInfo>,
}

fn collect_hardware() -> HardwareInfo {
    let wmi = collect_from_wmi();
    HardwareInfo {
        motherboard: wmi.motherboard,
        system_model: wmi.system_model,
        gpus: collect_gpus(),
        memory_modules: wmi.memory_modules,
    }
}

/// WMI 查询得到的主板 / 整机 / 内存条。
#[derive(Default)]
struct WmiHardware {
    motherboard: String,
    system_model: String,
    memory_modules: Vec<MemoryModuleInfo>,
}

/// WMI 依赖 COM，主线程可能已被以其它套间模型初始化，因此放到独立线程里查询。
#[cfg(windows)]
fn collect_from_wmi() -> WmiHardware {
    std::thread::spawn(query_wmi).join().unwrap_or_default()
}

#[cfg(not(windows))]
fn collect_from_wmi() -> WmiHardware {
    WmiHardware::default()
}

#[cfg(windows)]
fn query_wmi() -> WmiHardware {
    use wmi::{COMLibrary, WMIConnection};

    let Ok(com) = COMLibrary::new() else {
        return WmiHardware::default();
    };
    let Ok(connection) = WMIConnection::new(com) else {
        return WmiHardware::default();
    };

    let motherboard = connection
        .raw_query::<WmiBaseBoard>("SELECT Manufacturer, Product FROM Win32_BaseBoard")
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .map(|board| join_parts([board.manufacturer, board.product]))
        .unwrap_or_default();

    let system_model = connection
        .raw_query::<WmiSystemProduct>("SELECT Vendor, Version FROM Win32_ComputerSystemProduct")
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .map(|product| join_parts([product.vendor, product.version]))
        .unwrap_or_default();

    let memory_modules = connection
        .raw_query::<WmiMemory>(
            "SELECT Manufacturer, PartNumber, Capacity, Speed, SMBIOSMemoryType FROM Win32_PhysicalMemory",
        )
        .map(|rows| rows.into_iter().map(MemoryModuleInfo::from).collect())
        .unwrap_or_default();

    WmiHardware {
        motherboard,
        system_model,
        memory_modules,
    }
}

#[cfg(windows)]
fn join_parts<const N: usize>(parts: [Option<String>; N]) -> String {
    parts
        .into_iter()
        .flatten()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn memory_type_label(code: u32) -> String {
    match code {
        20 => "DDR",
        21 => "DDR2",
        24 => "DDR3",
        26 => "DDR4",
        34 => "DDR5",
        _ => "",
    }
    .to_string()
}

#[cfg(windows)]
#[derive(Deserialize)]
struct WmiBaseBoard {
    #[serde(rename = "Manufacturer")]
    manufacturer: Option<String>,
    #[serde(rename = "Product")]
    product: Option<String>,
}

#[cfg(windows)]
#[derive(Deserialize)]
struct WmiSystemProduct {
    #[serde(rename = "Vendor")]
    vendor: Option<String>,
    #[serde(rename = "Version")]
    version: Option<String>,
}

#[cfg(windows)]
#[derive(Deserialize)]
struct WmiMemory {
    #[serde(rename = "Manufacturer")]
    manufacturer: Option<String>,
    #[serde(rename = "PartNumber")]
    part_number: Option<String>,
    #[serde(rename = "Capacity")]
    capacity: Option<u64>,
    #[serde(rename = "Speed")]
    speed: Option<u32>,
    #[serde(rename = "SMBIOSMemoryType")]
    smbios_memory_type: Option<u32>,
}

#[cfg(windows)]
impl From<WmiMemory> for MemoryModuleInfo {
    fn from(raw: WmiMemory) -> Self {
        MemoryModuleInfo {
            manufacturer: raw.manufacturer.unwrap_or_default().trim().to_string(),
            part_number: raw.part_number.unwrap_or_default().trim().to_string(),
            capacity: raw.capacity.unwrap_or(0),
            speed: raw.speed.unwrap_or(0),
            memory_type: raw
                .smbios_memory_type
                .map(memory_type_label)
                .unwrap_or_default(),
        }
    }
}

/// 用 DXGI 枚举显卡：只看真实（非软件模拟）适配器，取型号与专用显存。
#[cfg(windows)]
fn collect_gpus() -> Vec<GpuInfo> {
    use std::collections::HashSet;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    let mut gpus = Vec::new();
    // 虚拟显示器（远控/投屏类软件）会把同一块显卡枚举成多个适配器，按硬件标识去重。
    let mut seen = HashSet::new();
    // SAFETY: 只读枚举本机显卡；任一步失败即提前返回，不 panic。
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return gpus;
        };

        let mut index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            // 跳过「Microsoft Basic Render Driver」之类的软件适配器。
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let name = utf16_to_string(&desc.Description);
            if name.is_empty() {
                continue;
            }
            let key = (desc.VendorId, desc.DeviceId, desc.SubSysId, desc.Revision);
            if !seen.insert(key) {
                continue;
            }
            gpus.push(GpuInfo {
                name,
                memory: desc.DedicatedVideoMemory as u64,
            });
        }
    }
    gpus
}

#[cfg(not(windows))]
fn collect_gpus() -> Vec<GpuInfo> {
    Vec::new()
}

#[cfg(windows)]
fn utf16_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

fn disk_kind_label(kind: DiskKind) -> &'static str {
    match kind {
        DiskKind::HDD => "HDD",
        DiskKind::SSD => "SSD",
        _ => "未知",
    }
}

/// 环回与虚拟网卡（Windows 上会额外列出），统计网络速率时需排除以免重复计数。
fn is_virtual_interface(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "loopback",
        "vethernet",
        "virtual",
        "vmware",
        "hyper-v",
        "bluetooth",
        "tap",
        "tun",
    ]
    .iter()
    .any(|keyword| name.contains(keyword))
}

fn has_routable_ipv4(data: &NetworkData) -> bool {
    data.ip_networks().iter().any(|network| match network.addr {
        IpAddr::V4(addr) => !addr.is_loopback() && !addr.is_link_local(),
        IpAddr::V6(_) => false,
    })
}

/// 取主网卡 IPv4：先用 UDP `connect` 让系统按路由表挑出源地址，失败再遍历网卡兜底。
fn primary_ipv4(networks: &Networks) -> String {
    if let Some(addr) = probe_local_ipv4() {
        return addr.to_string();
    }

    for (name, data) in networks.list() {
        if is_virtual_interface(name) {
            continue;
        }
        for network in data.ip_networks() {
            if let IpAddr::V4(addr) = network.addr {
                if !addr.is_loopback() && !addr.is_link_local() {
                    return addr.to_string();
                }
            }
        }
    }

    "未连接".to_string()
}

fn probe_local_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // UDP connect 只设置默认下一跳、不发送报文；local_addr 即路由表选出的源地址。
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(addr) if !addr.is_loopback() => Some(addr),
        _ => None,
    }
}
