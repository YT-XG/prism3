use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{DiskKind, Disks, NetworkData, Networks, System};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
use serde::Deserialize;

/// 磁盘列表的最小刷新间隔，降低采集开销。
const DISK_REFRESH_SECS: u64 = 5;

/// 托盘 tooltip 与阈值告警的采样间隔。5 秒足够及时，也比 1 秒轮询省 CPU。
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
/// 内存占用持续超过该值才告警（连续两个采样点，避免瞬时峰值）。
const ALERT_MEMORY_PERCENT: f32 = 90.0;
/// 磁盘已用超过该值才告警。
const ALERT_DISK_PERCENT: f32 = 90.0;
/// 同类告警的最短间隔，避免在阈值附近反复弹通知。
const ALERT_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// GPU 温度超过该值时告警（仅 NVIDIA 提供温度时生效）。
const ALERT_GPU_TEMP: f32 = 85.0;

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
    /// GPU 全局占用率（0-100，按最忙的引擎统计）；读不到时为 `None`。
    pub gpu_usage: Option<f32>,
    /// 主显卡专用显存的已用量 / 预算（字节，读不到时为 0）。
    pub gpu_memory_used: u64,
    pub gpu_memory_total: u64,
    /// GPU 温度（℃）与风扇转速（%）；只有 NVIDIA 驱动提供 `nvidia-smi` 时才有值。
    pub gpu_temperature: Option<f32>,
    pub gpu_fan_speed: Option<u32>,
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
    gpu_utilization: GpuUtilization,
    nvidia: NvidiaCache,
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
                gpu_utilization: GpuUtilization::new(),
                nvidia: NvidiaCache::new(),
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

        // GPU：PDH 统计所有引擎的占用（取最忙的引擎），DXGI 读主显卡显存预算；
        // 温度 / 风扇只有装了 NVIDIA 驱动时才能从 nvidia-smi 取到。
        let nvidia = inner.nvidia.get();
        let gpu_usage = inner
            .gpu_utilization
            .sample()
            .or_else(|| nvidia.as_ref().and_then(|sample| sample.utilization));
        let (memory_used, memory_total) = query_gpu_memory();
        let gpu_memory_used = if memory_used > 0 {
            memory_used
        } else {
            nvidia
                .as_ref()
                .map(|sample| sample.memory_used)
                .unwrap_or(0)
        };
        let gpu_memory_total = if memory_total > 0 {
            memory_total
        } else {
            nvidia
                .as_ref()
                .map(|sample| sample.memory_total)
                .unwrap_or(0)
        };
        let gpu_temperature = nvidia.as_ref().and_then(|sample| sample.temperature);
        let gpu_fan_speed = nvidia.as_ref().and_then(|sample| sample.fan_speed);

        SystemSnapshot {
            cpu_usage,
            mem_used: inner.system.used_memory(),
            mem_total: inner.system.total_memory(),
            net_rx_per_sec,
            net_tx_per_sec,
            uptime_secs: System::uptime(),
            disks,
            gpu_usage,
            gpu_memory_used,
            gpu_memory_total,
            gpu_temperature,
            gpu_fan_speed,
        }
    }

    /// 返回静态系统信息（克隆一份，前端取一次后自行缓存）。
    pub fn statics(&self) -> SystemStatics {
        self.statics.clone()
    }
}

/// GPU 占用率来源：Windows PDH 的 `GPU Engine` 计数器（不依赖厂商 SDK）。
#[cfg(windows)]
struct GpuUtilization {
    /// `PDH_HQUERY` 的原始指针；`Send` 约束要求存成整数（访问都在 `Mutex` 里）。
    query: isize,
    counter: isize,
    primed: bool,
}

#[cfg(windows)]
impl GpuUtilization {
    fn new() -> Self {
        use windows::core::HSTRING;
        use windows::Win32::System::Performance::{
            PdhAddEnglishCounterW, PdhCloseQuery, PdhOpenQueryW, PDH_HCOUNTER, PDH_HQUERY,
        };

        let mut query = PDH_HQUERY::default();
        // SAFETY: 打开一个本地 PDH 查询；失败时句柄保持空值，后续采样直接返回 None。
        let status = unsafe { PdhOpenQueryW(&HSTRING::from(""), 0, &mut query) };
        if status != 0 {
            return Self {
                query: 0,
                counter: 0,
                primed: false,
            };
        }

        let mut counter = PDH_HCOUNTER::default();
        let path = HSTRING::from(r"\GPU Engine(*)\Utilization Percentage");
        // 英文计数器名避免中文系统上本地化名称不一致。
        let status = unsafe { PdhAddEnglishCounterW(query, &path, 0, &mut counter) };
        if status != 0 {
            unsafe { PdhCloseQuery(query) };
            return Self {
                query: 0,
                counter: 0,
                primed: false,
            };
        }

        Self {
            query: query.0 as isize,
            counter: counter.0 as isize,
            primed: false,
        }
    }

    /// 采样一次；PDH 需要两个时间点才能算出速率，首次返回 `None`。
    fn sample(&mut self) -> Option<f32> {
        use windows::Win32::System::Performance::{
            PdhCollectQueryData, PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W,
            PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
        };

        if self.query == 0 || self.counter == 0 {
            return None;
        }
        let query = PDH_HQUERY(self.query as *mut _);
        let counter = PDH_HCOUNTER(self.counter as *mut _);

        // SAFETY: 句柄由 `new` 打开且在本结构生命周期内有效；缓冲区按 API 返回的
        // 字节数分配，第二次调用只写入这块内存；所有字符串读取都先检查空指针。
        unsafe {
            if PdhCollectQueryData(query) != 0 {
                return None;
            }
            if !self.primed {
                self.primed = true;
                return None;
            }

            let mut size = 0u32;
            let mut count = 0u32;
            let status =
                PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if status != PDH_MORE_DATA || size == 0 {
                return None;
            }
            let mut buffer = vec![0u8; size as usize];
            let status = PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
            );
            if status != 0 {
                return None;
            }

            let items = buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W;
            // 同一引擎上各进程的占用相加就是该引擎的总占用；再取最忙的引擎
            // （3D / Copy / VideoDecode…）作为「GPU 占用」，与任务管理器口径接近。
            let mut engines: HashMap<String, f64> = HashMap::new();
            for index in 0..count as usize {
                let item = &*items.add(index);
                if item.szName.0.is_null() || item.FmtValue.CStatus != 0 {
                    continue;
                }
                let name = pwstr_to_string(item.szName);
                let value = item.FmtValue.Anonymous.doubleValue;
                if !value.is_finite() || value <= 0.0 {
                    continue;
                }
                let Some(engine) = name.rsplit("engtype_").next() else {
                    continue;
                };
                if engine.is_empty() {
                    continue;
                }
                *engines.entry(engine.to_string()).or_default() += value;
            }
            let busiest = engines.values().copied().fold(0.0, f64::max);
            (busiest > 0.0).then(|| (busiest as f32).clamp(0.0, 100.0))
        }
    }
}

#[cfg(windows)]
impl Drop for GpuUtilization {
    fn drop(&mut self) {
        if self.query != 0 {
            use windows::Win32::System::Performance::{PdhCloseQuery, PDH_HQUERY};
            // SAFETY: 只关闭本结构持有的查询句柄。
            unsafe { PdhCloseQuery(PDH_HQUERY(self.query as *mut _)) };
        }
    }
}

#[cfg(not(windows))]
struct GpuUtilization;

#[cfg(not(windows))]
impl GpuUtilization {
    fn new() -> Self {
        Self
    }

    fn sample(&mut self) -> Option<f32> {
        None
    }
}

#[cfg(windows)]
unsafe fn pwstr_to_string(value: windows::core::PWSTR) -> String {
    if value.0.is_null() {
        return String::new();
    }
    let len = (0..).take_while(|&index| *value.0.add(index) != 0).count();
    String::from_utf16_lossy(std::slice::from_raw_parts(value.0, len))
}

/// 主显卡显存：DXGI `QueryVideoMemoryInfo`（本地段 = 专用显存）。
#[cfg(windows)]
fn query_gpu_memory() -> (u64, u64) {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    // SAFETY: 只读枚举适配器并查询显存预算；任一步失败即返回 0，不影响其它监控项。
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return (0, 0);
        };
        let mut index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() else {
                continue;
            };
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if adapter3
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
                .is_ok()
            {
                let total = if info.Budget > 0 {
                    info.Budget
                } else {
                    desc.DedicatedVideoMemory as u64
                };
                return (info.CurrentUsage, total);
            }
        }
    }
    (0, 0)
}

#[cfg(not(windows))]
fn query_gpu_memory() -> (u64, u64) {
    (0, 0)
}

#[derive(Clone, Default)]
struct NvidiaSample {
    utilization: Option<f32>,
    temperature: Option<f32>,
    fan_speed: Option<u32>,
    memory_used: u64,
    memory_total: u64,
}

/// `nvidia-smi` 每次调用约几十毫秒：有值就缓存 5 秒，没装驱动时隔 30 秒再试。
struct NvidiaCache {
    sample: Option<NvidiaSample>,
    last_attempt: Option<Instant>,
}

impl NvidiaCache {
    fn new() -> Self {
        Self {
            sample: None,
            last_attempt: None,
        }
    }

    fn get(&mut self) -> Option<NvidiaSample> {
        let due = self
            .last_attempt
            .map(|last| {
                last.elapsed()
                    >= if self.sample.is_some() {
                        Duration::from_secs(5)
                    } else {
                        Duration::from_secs(30)
                    }
            })
            .unwrap_or(true);
        if due {
            self.last_attempt = Some(Instant::now());
            self.sample = query_nvidia();
        }
        self.sample.clone()
    }
}

#[cfg(windows)]
fn query_nvidia() -> Option<NvidiaSample> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,temperature.gpu,fan.speed,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() < 5 {
        return None;
    }
    Some(NvidiaSample {
        utilization: fields[0].parse().ok(),
        temperature: fields[1].parse().ok(),
        fan_speed: fields[2].parse().ok(),
        memory_used: fields[3]
            .parse::<u64>()
            .map(|mib| mib.saturating_mul(1024 * 1024))
            .unwrap_or(0),
        memory_total: fields[4]
            .parse::<u64>()
            .map(|mib| mib.saturating_mul(1024 * 1024))
            .unwrap_or(0),
    })
}

#[cfg(not(windows))]
fn query_nvidia() -> Option<NvidiaSample> {
    None
}

/// 后台看门狗：每 5 秒采一次负载，更新托盘 tooltip，并在内存 / 磁盘 / GPU 温度
/// 超过阈值时发系统通知。只在应用运行期间存在一条线程，开销固定。
pub fn spawn_watchdog(app: AppHandle) {
    std::thread::spawn(move || {
        // 启动阶段先让主窗口与数据库初始化完成，避免和启动维护抢 CPU。
        std::thread::sleep(Duration::from_secs(5));
        let mut memory_streak = 0u8;
        let mut last_alert: HashMap<String, Instant> = HashMap::new();

        loop {
            let snapshot = app.state::<MonitorState>().snapshot();
            update_tray_tooltip(&app, &snapshot);

            let memory_percent = if snapshot.mem_total > 0 {
                snapshot.mem_used as f32 / snapshot.mem_total as f32 * 100.0
            } else {
                0.0
            };
            if memory_percent >= ALERT_MEMORY_PERCENT {
                memory_streak = memory_streak.saturating_add(1);
            } else {
                memory_streak = 0;
            }
            if memory_streak >= 2 {
                alert(
                    &app,
                    &mut last_alert,
                    "memory",
                    format!("内存占用持续偏高（{memory_percent:.0}%），可以检查一下后台进程。"),
                );
            }

            for disk in &snapshot.disks {
                let used = disk.total.saturating_sub(disk.available);
                let percent = if disk.total > 0 {
                    used as f32 / disk.total as f32 * 100.0
                } else {
                    0.0
                };
                if percent >= ALERT_DISK_PERCENT {
                    alert(
                        &app,
                        &mut last_alert,
                        &format!("disk:{}", disk.mount_point),
                        format!(
                            "磁盘 {} 已用 {percent:.0}%，剩余空间不足 10%。",
                            disk.mount_point
                        ),
                    );
                }
            }

            if let Some(temperature) = snapshot.gpu_temperature {
                if temperature >= ALERT_GPU_TEMP {
                    alert(
                        &app,
                        &mut last_alert,
                        "gpu-temp",
                        format!("GPU 温度达到 {temperature:.0}℃。"),
                    );
                }
            }

            std::thread::sleep(WATCHDOG_INTERVAL);
        }
    });
}

fn update_tray_tooltip(app: &AppHandle, snapshot: &SystemSnapshot) {
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };
    let cpu = snapshot
        .cpu_usage
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| "—".to_string());
    let memory = if snapshot.mem_total > 0 {
        format!(
            "{:.0}%",
            snapshot.mem_used as f32 / snapshot.mem_total as f32 * 100.0
        )
    } else {
        "—".to_string()
    };
    let mut text = format!("Prism3 · CPU {cpu} · 内存 {memory}");
    if let Some(gpu) = snapshot.gpu_usage {
        text.push_str(&format!(" · GPU {gpu:.0}%"));
    }
    if let Err(err) = tray.set_tooltip(Some(text)) {
        log::debug!("更新托盘 tooltip 失败：{err}");
    }
}

fn alert(app: &AppHandle, last_alert: &mut HashMap<String, Instant>, key: &str, message: String) {
    let due = last_alert
        .get(key)
        .map(|last| last.elapsed() >= ALERT_COOLDOWN)
        .unwrap_or(true);
    if !due {
        return;
    }
    last_alert.insert(key.to_string(), Instant::now());

    if let Err(err) = app
        .notification()
        .builder()
        .title("Prism3 系统提醒")
        .body(&message)
        .show()
    {
        log::warn!("发送系统通知失败：{err}");
    }
    if let Err(err) = app.emit(
        "monitor:alert",
        AlertPayload {
            kind: key.to_string(),
            message,
        },
    ) {
        log::warn!("广播监控告警失败：{err}");
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlertPayload {
    kind: String,
    message: String,
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn gpu_sampling_does_not_panic_without_a_gpu() {
        // PDH 计数器不是每台机器都有（虚拟机 / 无显卡环境）：只要求不 panic、失败时返回 None。
        let mut gpu = GpuUtilization::new();
        let _ = gpu.sample();
        std::thread::sleep(Duration::from_millis(50));
        let _ = gpu.sample();
        let _ = query_gpu_memory();
    }
}
