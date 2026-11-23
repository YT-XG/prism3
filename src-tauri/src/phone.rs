//! 手机（Android）远程控制：adb 截屏 + 触摸 / 按键注入。
//!
//! 设计取舍：
//!
//! - **只依赖 adb**。不做 scrcpy 那一套（要额外推 scrcpy-server、解 H.264、再解码），
//!   截图轮询的实测开销（Redmi K90 Pro Max，1200×2608，USB）：
//!   低细节画面约 0.6 s/帧（PNG ~1.1 MB）、带壁纸的主屏约 1.3 s/帧（PNG ~3.5 MB），
//!   也就是 **0.7–3 fps**，随画面复杂度变化。点按 / 滑动本身只要 ~70–90 ms，
//!   所以「控制是实时的、预览是低频的」——这个取舍写在这里，别为了「更流畅」悄悄引入视频管线。
//! - 每帧用 `exec-out screencap -p` 取**原始 PNG 字节**，经 `tauri::ipc::Response`
//!   直接作为 `ArrayBuffer` 交给前端（见 tauri `scripts/ipc-protocol.js`：非 JSON
//!   响应走 `response.arrayBuffer()`）。刻意不走 base64：一张 1–3 MB 的图 base64 后
//!   再胖 1/3，且每帧都要在 IPC 上过一次字符串。
//! - adb 来源按序回落：配置 `phoneAdbPath` → 应用数据目录里下载的 platform-tools →
//!   PATH → 常见 SDK 安装位置。都没有时页面给「一键下载」（见 `install_platform_tools`）。
//! - **不做静默失败**：截屏拿到的不是 PNG 就报错，锁屏读不到屏幕状态就明说「无法确认」，
//!   绝不把「指令发了」说成「已经锁了」。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::config;
use crate::error::{AppError, AppResult};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// 不创建控制台窗口（Windows `CREATE_NO_WINDOW`）：adb 是控制台程序，
/// 不加这个每调一次就闪一下黑框（与 `projects.rs` 同一处理）。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Google 官方 platform-tools 压缩包（Windows）。约 8 MB，解压出 adb.exe + 两个 dll。
const PLATFORM_TOOLS_URL: &str =
    "https://dl.google.com/android/repository/platform-tools-latest-windows.zip";

/// 下载上限（官方包 8 MB 左右，给足余量同时挡住异常响应把磁盘写满）。
const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

/// PNG 文件头。`exec-out` 正常是二进制安全的，但老 adb / 个别设备会把 `\n` 翻成
/// `\r\n`，头被破坏后前端只会得到一张坏图——所以要显式校验。
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// `adb devices` 可能要现场拉起 adb server（首次 1–2 秒）。
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
/// 截屏：`screencap` 本身 100–300 ms，再加上 1–3 MB 的传输，留足。
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(15);
/// 注入一条 `input` 命令。
const INPUT_TIMEOUT: Duration = Duration::from_secs(8);
/// `dumpsys power` 读屏幕状态。
const DUMP_TIMEOUT: Duration = Duration::from_secs(8);

/// 一台通过 adb 看到的 Android 设备。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneDevice {
    pub serial: String,
    /// adb 报的状态：`device`（已授权）/ `unauthorized` / `offline` / `no permissions` …
    pub state: String,
    /// 设备型号（来自 `devices -l` 的 `model:`；取不到时为空串）。
    pub model: String,
    /// 产品名（来自 `product:`；取不到时为空串）。
    pub product: String,
    /// 只有 `device` 状态才表示「已授权、可以截屏与注入」。
    pub authorized: bool,
}

/// 手机页需要的全部环境信息（一次性返回，避免前端为每个字段各调一次命令）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneStatus {
    pub adb_found: bool,
    pub adb_path: String,
    /// `config` / `bundled` / `path` / `sdk`；没找到时为空串。
    pub adb_source: String,
    pub devices: Vec<PhoneDevice>,
    /// 前端应该使用的设备序列号：配置里记住的 → 第一台已授权 → 第一台。
    pub selected: Option<String>,
    /// 本次探测的失败原因（没找到 adb / adb devices 报错）；成功时为 `null`。
    pub error: Option<String>,
}

/// 前端可以直接点的按键。只收白名单里的名字，**不接受裸 keycode**
/// （裸数字等于给前端开了「任意按键」的口子，也没有可读的错误信息）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhoneKey {
    Back,
    Home,
    Recents,
    Power,
    Wake,
    Sleep,
    VolumeUp,
    VolumeDown,
}

impl PhoneKey {
    fn code(self) -> u32 {
        match self {
            // Android KeyEvent 常量。
            PhoneKey::Back => 4,
            PhoneKey::Home => 3,
            PhoneKey::Recents => 187,
            PhoneKey::Power => 26,
            PhoneKey::Wake => 224,
            PhoneKey::Sleep => 223,
            PhoneKey::VolumeUp => 24,
            PhoneKey::VolumeDown => 25,
        }
    }
}

/// 已解析到的 adb 可执行文件及来源。
pub struct ResolvedAdb {
    pub path: PathBuf,
    /// `config` / `bundled` / `path` / `sdk`，用于在页面上说清「用的是哪一个 adb」。
    pub source: &'static str,
}

/// 一条 adb 调用的结果。
struct AdbOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl AdbOutput {
    /// stderr 可能带整段帮助文本，报错时只取一行摘要。
    fn error_tail(&self) -> String {
        let text = self.stderr.trim();
        if text.is_empty() {
            return "没有返回错误信息".to_string();
        }
        let line = text.lines().last().unwrap_or(text).trim();
        if line.chars().count() > 240 {
            let cut: String = line.chars().take(240).collect();
            return format!("{cut}…");
        }
        line.to_string()
    }
}

// ---------------------------------------------------------------------------
// 执行 adb
// ---------------------------------------------------------------------------

/// 跑一次子进程，带**硬超时**。
///
/// 不能用 `Command::output()`：设备掉线时 adb 客户端会一直等，UI 就永远转圈。
/// 这里自己收管道（两个读线程避免缓冲区写满把子进程卡住），到点 `kill` 并报错。
fn run_process(program: &Path, args: &[String], timeout: Duration) -> AppResult<AdbOutput> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|err| AppError::Other(format!("启动 {} 失败：{err}", program.display())))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Other("读取 adb 输出失败".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Other("读取 adb 输出失败".into()))?;
    let stdout_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer);
        buffer
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(AppError::Other(format!(
                        "adb 无响应（超过 {} 秒），已终止本次调用",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                let _ = child.kill();
                return Err(AppError::Other(format!("等待 adb 结束失败：{err}")));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr_thread.join().unwrap_or_default())
        .trim()
        .to_string();
    Ok(AdbOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
// adb 定位
// ---------------------------------------------------------------------------

fn config_snapshot(app: &AppHandle) -> config::AppConfig {
    app.state::<config::AppState>()
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 应用数据目录下自带的那份 adb（`install_platform_tools` 解压到这里）。
pub fn bundled_adb_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?.join("platform-tools");
    let path = dir.join("adb.exe");
    path.is_file().then_some(path)
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn sdk_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("ANDROID_HOME") {
        dirs.push(PathBuf::from(home).join("platform-tools").join("adb.exe"));
    }
    if let Some(root) = std::env::var_os("ANDROID_SDK_ROOT") {
        dirs.push(PathBuf::from(root).join("platform-tools").join("adb.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(local)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join("adb.exe"),
        );
    }
    dirs
}

/// 解析当前该用哪个 adb：配置 → 自带 → PATH → SDK。
pub fn resolve_adb(app: &AppHandle) -> Option<ResolvedAdb> {
    let configured = config_snapshot(app).phone_adb_path;
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Some(ResolvedAdb {
                path,
                source: "config",
            });
        }
        // 配置了但文件没了：继续往下回落，不因为一个过期路径就整页不可用。
        log::warn!("配置的 adb 路径不存在，改用自动探测：{configured}");
    }
    if let Some(path) = bundled_adb_path(app) {
        return Some(ResolvedAdb {
            path,
            source: "bundled",
        });
    }
    if let Some(path) = which("adb.exe").or_else(|| which("adb")) {
        return Some(ResolvedAdb {
            path,
            source: "path",
        });
    }
    sdk_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|path| ResolvedAdb {
            path,
            source: "sdk",
        })
}

fn require_adb(app: &AppHandle) -> AppResult<ResolvedAdb> {
    resolve_adb(app).ok_or_else(|| {
        AppError::Other("没找到 adb：先在页面里下载 platform-tools，或手动指定 adb.exe 路径".into())
    })
}

// ---------------------------------------------------------------------------
// 设备列表
// ---------------------------------------------------------------------------

/// 解析 `adb devices -l` 的输出。
///
/// 真实格式（`-l` 会带一长串 `key:value`）：
///
/// ```text
/// List of devices attached
/// d989934a               device usb:1-1 product:marble model:redmi_k90_pro_max device:marble
/// 127.0.0.1:5555         offline
/// emulator-5554          unauthorized
/// ```
pub fn parse_devices(text: &str) -> Vec<PhoneDevice> {
    let mut devices = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with('*')
            || line.starts_with("adb server")
        {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(serial) = tokens.next() else {
            continue;
        };
        let Some(state) = tokens.next() else {
            continue;
        };
        // `no permissions (...)` 这类状态在 adb 输出里是三个词，拼回去保持可读。
        let mut state = state.to_string();
        let mut properties: Vec<&str> = Vec::new();
        for token in tokens {
            if token.contains(':') {
                properties.push(token);
            } else if state == "no" {
                state.push(' ');
                state.push_str(token);
            }
        }
        let lookup = |key: &str| {
            properties
                .iter()
                .find_map(|entry| entry.strip_prefix(key)?.strip_prefix(':'))
                .unwrap_or_default()
                .to_string()
        };
        devices.push(PhoneDevice {
            serial: serial.to_string(),
            authorized: state == "device",
            model: lookup("model"),
            product: lookup("product"),
            state,
        });
    }
    devices
}

/// 选一台设备：配置里记住的（且已授权）→ 第一台已授权 → 第一台。
fn pick_serial(app: &AppHandle, devices: &[PhoneDevice]) -> Option<String> {
    if devices.is_empty() {
        return None;
    }
    let remembered = config_snapshot(app).phone_serial;
    if !remembered.is_empty() {
        if let Some(device) = devices
            .iter()
            .find(|device| device.authorized && device.serial == remembered)
        {
            return Some(device.serial.clone());
        }
    }
    if let Some(device) = devices.iter().find(|device| device.authorized) {
        return Some(device.serial.clone());
    }
    devices.first().map(|device| device.serial.clone())
}

pub fn status(app: &AppHandle) -> PhoneStatus {
    let Some(adb) = resolve_adb(app) else {
        return PhoneStatus {
            adb_found: false,
            adb_path: String::new(),
            adb_source: String::new(),
            devices: Vec::new(),
            selected: None,
            error: Some(
                "没找到 adb：应用内下载 platform-tools，或手动指定 adb.exe 路径".to_string(),
            ),
        };
    };

    let args = vec!["devices".to_string(), "-l".to_string()];
    let (devices, error) = match run_process(&adb.path, &args, STATUS_TIMEOUT) {
        Ok(output) if output.success => (
            parse_devices(&String::from_utf8_lossy(&output.stdout)),
            None,
        ),
        Ok(output) => (
            Vec::new(),
            Some(format!("adb devices 失败：{}", output.error_tail())),
        ),
        Err(err) => (Vec::new(), Some(err.to_string())),
    };
    let selected = pick_serial(app, &devices);

    PhoneStatus {
        adb_found: true,
        adb_path: adb.path.display().to_string(),
        adb_source: adb.source.to_string(),
        devices,
        selected,
        error,
    }
}

/// 记住用户选中的设备（写配置，下次打开手机页直接选中它）。
pub fn select_device(app: &AppHandle, serial: &str) -> AppResult<()> {
    let serial = checked_serial(serial)?;
    let state = app.state::<config::AppState>();
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.phone_serial == serial {
        return Ok(());
    }
    guard.phone_serial = serial.to_string();
    let snapshot = guard.clone();
    drop(guard);
    config::save(app, &snapshot)
}

// ---------------------------------------------------------------------------
// 截屏
// ---------------------------------------------------------------------------

fn checked_serial(serial: &str) -> AppResult<String> {
    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::Other("还没有选择设备".into()));
    }
    if serial.chars().count() > 128 || serial.chars().any(|ch| ch.is_whitespace() || ch == '\u{0}')
    {
        return Err(AppError::Other(format!("设备序列号非法：{serial}")));
    }
    Ok(serial.to_string())
}

fn base_args(serial: &str) -> Vec<String> {
    vec!["-s".to_string(), serial.to_string()]
}

/// 校验（并在必要时修复）PNG 头。
fn normalize_png(bytes: Vec<u8>) -> AppResult<Vec<u8>> {
    if bytes.starts_with(&PNG_MAGIC) {
        return Ok(bytes);
    }
    // 只修「CRLF 翻倍」这一种可逆破坏：把每处 `\r\n` 收回 `\n` 后再判一次头。
    let mut repaired = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        repaired.push(bytes[index]);
        index += 1;
    }
    if repaired.starts_with(&PNG_MAGIC) {
        log::warn!("截屏数据换行被改写（CRLF），已按 PNG 头修复后使用");
        return Ok(repaired);
    }
    Err(AppError::Other(
        "截屏返回的不是 PNG：设备可能处于黑屏、或 adb 版本异常".into(),
    ))
}

pub fn screencap(app: &AppHandle, serial: &str) -> AppResult<Vec<u8>> {
    let serial = checked_serial(serial)?;
    let adb = require_adb(app)?;

    let mut args = base_args(&serial);
    args.extend(["exec-out", "screencap", "-p"].map(String::from));
    let output = run_process(&adb.path, &args, SCREENSHOT_TIMEOUT)?;
    if !output.success {
        return Err(AppError::Other(format!(
            "截屏失败：{}",
            output.error_tail()
        )));
    }
    normalize_png(output.stdout)
}

// ---------------------------------------------------------------------------
// 触摸 / 按键注入
// ---------------------------------------------------------------------------

fn run_shell(adb: &Path, serial: &str, shell_args: &[String], label: &str) -> AppResult<()> {
    let mut args = base_args(serial);
    args.push("shell".to_string());
    args.extend(shell_args.iter().cloned());
    let output = run_process(adb, &args, INPUT_TIMEOUT)?;
    if !output.success {
        return Err(AppError::Other(format!(
            "{label}失败：{}",
            output.error_tail()
        )));
    }
    Ok(())
}

pub fn tap(app: &AppHandle, serial: &str, x: i32, y: i32) -> AppResult<()> {
    let serial = checked_serial(serial)?;
    let adb = require_adb(app)?;
    let args = vec![
        "input".to_string(),
        "tap".to_string(),
        x.to_string(),
        y.to_string(),
    ];
    run_shell(&adb.path, &serial, &args, "点击")
}

pub fn swipe(
    app: &AppHandle,
    serial: &str,
    from: (i32, i32),
    to: (i32, i32),
    duration_ms: u32,
) -> AppResult<()> {
    let serial = checked_serial(serial)?;
    let adb = require_adb(app)?;
    // 太短的滑动在部分 ROM 上会被当成点击；太长的滑动没必要（拖拽手感）。
    let duration = duration_ms.clamp(40, 2000);
    let args = vec![
        "input".to_string(),
        "swipe".to_string(),
        from.0.to_string(),
        from.1.to_string(),
        to.0.to_string(),
        to.1.to_string(),
        duration.to_string(),
    ];
    run_shell(&adb.path, &serial, &args, "滑动")
}

fn send_key(adb: &Path, serial: &str, code: u32) -> AppResult<()> {
    let args = vec![
        "input".to_string(),
        "keyevent".to_string(),
        code.to_string(),
    ];
    run_shell(adb, serial, &args, "按键")
}

pub fn key(app: &AppHandle, serial: &str, key: PhoneKey) -> AppResult<()> {
    let serial = checked_serial(serial)?;
    let adb = require_adb(app)?;
    send_key(&adb.path, &serial, key.code())
}

/// 读「屏幕是不是灭的」：`Some(true)` 灭屏、`Some(false)` 亮屏、`None` 读不到。
fn screen_asleep(adb: &Path, serial: &str) -> Option<bool> {
    let mut args = base_args(serial);
    args.extend(["shell", "dumpsys", "power"].map(String::from));
    let output = run_process(adb, &args, DUMP_TIMEOUT).ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    // Android 9+ 用 `mWakefulness=Awake|Asleep|Dozing`。
    if let Some(index) = text.find("mWakefulness=") {
        let rest = &text[index + "mWakefulness=".len()..];
        let word = rest
            .split(|ch: char| !ch.is_ascii_alphabetic())
            .next()
            .unwrap_or_default();
        return Some(matches!(word, "Asleep" | "Dozing"));
    }
    // 老系统用 `mScreenOn=true|false`。
    if let Some(index) = text.find("mScreenOn=") {
        let rest = &text[index + "mScreenOn=".len()..];
        return Some(rest.starts_with("false"));
    }
    None
}

/// 轮询等屏幕灭掉（最多 `limit`）。
fn wait_asleep(adb: &Path, serial: &str, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if screen_asleep(adb, serial) == Some(true) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// 锁屏，并**尽量验证**结果。
///
/// `KEYCODE_SLEEP`(223) 是语义正确的「熄屏」；部分 ROM 不认，才退到 `KEYCODE_POWER`(26)。
/// 关键是：**读不到屏幕状态时绝不发 26**——如果 223 其实已经生效，再按 26 就把屏幕点亮了。
pub fn lock(app: &AppHandle, serial: &str) -> AppResult<String> {
    let serial = checked_serial(serial)?;
    let adb = require_adb(app)?;

    match screen_asleep(&adb.path, &serial) {
        Some(true) => Ok("屏幕已经是熄灭的".to_string()),
        Some(false) => {
            send_key(&adb.path, &serial, PhoneKey::Sleep.code())?;
            if wait_asleep(&adb.path, &serial, Duration::from_millis(1500)) {
                return Ok("已锁屏".to_string());
            }
            send_key(&adb.path, &serial, PhoneKey::Power.code())?;
            if wait_asleep(&adb.path, &serial, Duration::from_millis(1500)) {
                return Ok("已锁屏（电源键）".to_string());
            }
            Err(AppError::Other(
                "锁屏指令已发送，但屏幕仍然是亮的：这台机器可能限制了按键注入".into(),
            ))
        }
        None => {
            // 读不到状态：只发语义正确的 223，不拿 26 赌。
            send_key(&adb.path, &serial, PhoneKey::Sleep.code())?;
            Ok("已发送锁屏指令（设备没有上报屏幕状态，无法确认）".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 下载 platform-tools
// ---------------------------------------------------------------------------

/// 下载并解压 platform-tools 到应用数据目录，返回可用的 adb。
///
/// 解压用 **Windows 10 自带的 `tar.exe`（bsdtar，能读 zip）**，只为解一个 zip 不值得
/// 往依赖里加一个 zip crate。解压先落临时目录再整体改名，避免半途失败留下半个
/// platform-tools。
pub fn install_platform_tools(app: &AppHandle) -> AppResult<ResolvedAdb> {
    let config_dir = app.path().app_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let target = config_dir.join("platform-tools");
    let staging = config_dir.join(".platform-tools-staging");
    let archive = config_dir.join("platform-tools-latest.zip");

    log::info!("开始下载 platform-tools（{PLATFORM_TOOLS_URL}）");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|err| AppError::Other(format!("创建下载请求失败：{err}")))?;
    let mut response = client
        .get(PLATFORM_TOOLS_URL)
        .send()
        .map_err(|err| AppError::Other(format!("下载 platform-tools 失败：{err}")))?;
    if !response.status().is_success() {
        return Err(AppError::Other(format!(
            "下载 platform-tools 失败：HTTP {}",
            response.status()
        )));
    }
    let downloaded = {
        let mut file = std::fs::File::create(&archive)?;
        let copied = std::io::copy(&mut response.by_ref().take(DOWNLOAD_LIMIT), &mut file)?;
        if copied == 0 {
            return Err(AppError::Other(
                "下载 platform-tools 失败：没有收到数据".into(),
            ));
        }
        copied
    };
    log::info!("platform-tools 下载完成：{downloaded} 字节，开始解压");

    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let args = vec![
        "-xf".to_string(),
        archive.display().to_string(),
        "-C".to_string(),
        staging.display().to_string(),
    ];
    let extracted = run_process(Path::new("tar"), &args, Duration::from_secs(120))
        .map_err(|err| AppError::Other(format!("解压 platform-tools 失败：{err}")))?;
    if !extracted.success {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::Other(format!(
            "解压 platform-tools 失败：{}",
            extracted.error_tail()
        )));
    }

    // zip 里带一层 `platform-tools/` 目录，把它整体搬到目标位置。
    let inner = staging.join("platform-tools");
    if !inner.join("adb.exe").is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::Other(
            "解压后的 platform-tools 里没有 adb.exe（压缩包结构异常）".into(),
        ));
    }
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(&inner, &target)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);

    let adb = target.join("adb.exe");
    if !adb.is_file() {
        return Err(AppError::Other(
            "platform-tools 安装后没找到 adb.exe".into(),
        ));
    }
    log::info!("platform-tools 安装完成：{}", adb.display());
    Ok(ResolvedAdb {
        path: adb,
        source: "bundled",
    })
}

// ---------------------------------------------------------------------------
// 无线调试（adb over Wi-Fi）
// ---------------------------------------------------------------------------

/// `adb tcpip` 用的端口（Android 的「无线调试」页面是另一个随机端口，两种都能用；
/// 固定 5555 是因为它是 `adb tcpip` 的约定值，切一次就一直有效，直到重启手机）。
pub const WIRELESS_PORT: u16 = 5555;
/// 切 tcpip / connect / disconnect 的硬超时（`adb connect` 连不上时会干等网络超时）。
const WIRELESS_TIMEOUT: Duration = Duration::from_secs(12);
/// `adb connect` 之后等设备真正进 `device` 状态的上限。
const WIRELESS_SETTLE: Duration = Duration::from_secs(8);

/// 无线调试的现状（手机页「无线」对话框用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WirelessInfo {
    /// 当前 USB 连着的那台（读 Wi-Fi IP、把 adbd 切到 TCP 都靠它）。
    pub usb_serial: Option<String>,
    /// 手机 `wlan0` 的局域网地址；没连 Wi-Fi 时是 None。
    pub ip: Option<String>,
    /// adbd 是否已经在监听 TCP（`service.adb.tcp.port`）；None = 还是 USB-only。
    pub tcp_port: Option<u16>,
    /// `adb devices` 里已经有 `ip:端口` 且状态是 `device`。
    pub connected: bool,
    /// 建议连的地址：已连接的优先 → 上次记住的 → `ip:5555`。
    pub address: Option<String>,
}

/// 从 `ip -f inet addr show wlan0` 的输出里取第一个 IPv4（去掉 `/24` 后缀）。
fn parse_wifi_ip(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("inet ") {
            let addr = rest.split_whitespace().next()?;
            let ip = addr.split('/').next().unwrap_or(addr);
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }
    None
}

/// `getprop service.adb.tcp.port` 的输出 → 端口（`0` / 空 = 没监听）。
fn parse_tcp_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port != 0)
}

/// `adb connect` 的地址校验：只收 `ip:端口`（或主机名），端口 1–65535。
///
/// 这个串会直接进 adb 的参数表，所以**必须**挡掉空白与控制字符——
/// 否则等于把「用户输入」拼进命令行。
fn checked_address(address: &str) -> AppResult<String> {
    let address = address.trim();
    let Some((host, port)) = address.rsplit_once(':') else {
        return Err(AppError::Other(
            "地址要写成「手机IP:端口」，例如 192.168.1.20:5555".into(),
        ));
    };
    let host = host.trim();
    if host.is_empty()
        || host.chars().count() > 64
        || host
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '\u{0}' || ch == '/' || ch == '\\')
    {
        return Err(AppError::Other(format!("地址里的 IP 不合法：{address}")));
    }
    let port: u16 = port
        .trim()
        .parse()
        .map_err(|_| AppError::Other(format!("端口不合法：{address}")))?;
    if port == 0 {
        return Err(AppError::Other("端口不能是 0".into()));
    }
    Ok(format!("{host}:{port}"))
}

/// 在设备上跑一条 shell 命令，只取 stdout（失败/超时一律 None，调用方按「不知道」处理）。
fn shell_text(adb: &Path, serial: &str, script: &str) -> Option<String> {
    let mut args = base_args(serial);
    args.push("shell".to_string());
    args.push(script.to_string());
    let output = run_process(adb, &args, STATUS_TIMEOUT).ok()?;
    output
        .success
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn read_wifi_ip(adb: &Path, serial: &str) -> Option<String> {
    shell_text(adb, serial, "ip -f inet addr show wlan0").and_then(|text| parse_wifi_ip(&text))
}

fn read_tcp_port(adb: &Path, serial: &str) -> Option<u16> {
    shell_text(adb, serial, "getprop service.adb.tcp.port").and_then(|text| parse_tcp_port(&text))
}

fn list_devices(adb: &Path) -> Vec<PhoneDevice> {
    let args = vec!["devices".to_string(), "-l".to_string()];
    match run_process(adb, &args, STATUS_TIMEOUT) {
        Ok(output) => parse_devices(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

/// 纯函数：把「设备列表 + 记住的地址」折成 UI 需要的现状（好测）。
fn build_wireless_info(adb: &Path, devices: &[PhoneDevice], remembered: &str) -> WirelessInfo {
    // 串里带 `:` 的就是 TCP 设备（`adb devices` 对 USB 设备给的是纯序列号）。
    let usb = devices
        .iter()
        .find(|device| device.authorized && !device.serial.contains(':'))
        .map(|device| device.serial.clone());
    let tcp = devices
        .iter()
        .find(|device| device.authorized && device.serial.contains(':'))
        .map(|device| device.serial.clone());
    let tcp_parts = tcp
        .as_deref()
        .and_then(|serial| serial.rsplit_once(':'))
        .map(|(host, port)| (host.to_string(), port.parse::<u16>().ok()));

    // 没插线时读不到手机上的东西（没有设备可 `adb shell`），但**连接地址里就有 IP 与端口**。
    // 早先这里只认 USB 读出来的值，于是「已经通过 Wi-Fi 连着」的界面却显示
    // 「手机 IP：无 / adbd 端口：USB 模式」——自相矛盾。
    let ip = usb
        .as_deref()
        .and_then(|serial| read_wifi_ip(adb, serial))
        .or_else(|| {
            tcp_parts
                .as_ref()
                .map(|(host, _)| host.clone())
                .filter(|host| !host.is_empty())
        });
    let tcp_port = usb
        .as_deref()
        .and_then(|serial| read_tcp_port(adb, serial))
        .or_else(|| tcp_parts.as_ref().and_then(|(_, port)| *port));
    let address = tcp
        .clone()
        .or_else(|| (!remembered.is_empty()).then(|| remembered.to_string()))
        .or_else(|| ip.as_ref().map(|ip| format!("{ip}:{WIRELESS_PORT}")));
    WirelessInfo {
        usb_serial: usb,
        ip,
        tcp_port,
        connected: tcp.is_some(),
        address,
    }
}

/// 读出无线调试现状（不产生任何副作用，可以随时轮询）。
pub fn wireless_info(app: &AppHandle) -> WirelessInfo {
    let Some(adb) = resolve_adb(app) else {
        return WirelessInfo::default();
    };
    let devices = list_devices(&adb.path);
    build_wireless_info(
        &adb.path,
        &devices,
        &config_snapshot(app).phone_wireless_addr,
    )
}

fn run_connect(adb: &Path, address: &str) -> AppResult<()> {
    let args = vec!["connect".to_string(), address.to_string()];
    let output = run_process(adb, &args, WIRELESS_TIMEOUT)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stdout.is_empty() {
        output.error_tail()
    } else {
        stdout.clone()
    };
    // `adb connect` 连不上时**可能仍然退出码 0**，只能看文案。
    if !output.success
        || message.contains("unable to connect")
        || message.contains("failed to connect")
        || message.contains("cannot connect")
    {
        return Err(AppError::Other(format!("连不上 {address}：{message}")));
    }

    // 连上不等于可用：等它真的出现在 `adb devices` 里且是 `device`。
    let deadline = Instant::now() + WIRELESS_SETTLE;
    loop {
        if list_devices(adb)
            .iter()
            .any(|device| device.serial == address && device.authorized)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::Other(format!(
                "{address} 能握手但没进 device 状态：手机端可能弹了授权框、或调试密钥不被信任"
            )));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// 记住这次连的地址，并把它设成当前设备（拔线之后镜像还能继续用）。
fn remember_wireless(app: &AppHandle, address: &str) {
    let state = app.state::<config::AppState>();
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.phone_wireless_addr = address.to_string();
    guard.phone_serial = address.to_string();
    let snapshot = guard.clone();
    drop(guard);
    if let Err(err) = config::save(app, &snapshot) {
        log::warn!("保存无线调试地址失败：{err}");
    }
}

/// 一键切到无线：把 adbd 切到 TCP（端口固定 5555），然后立刻连上。
///
/// 需要手机已经有局域网 IP；连的是移动数据时**明说做不到**，而不是给一个连不上的地址。
pub fn wireless_enable(app: &AppHandle) -> AppResult<WirelessInfo> {
    let adb = require_adb(app)?;
    let devices = list_devices(&adb.path);
    let usb = devices
        .iter()
        .find(|device| device.authorized && !device.serial.contains(':'))
        .map(|device| device.serial.clone())
        .ok_or_else(|| {
            AppError::Other("先用数据线连上手机，才能把它的 adbd 切到无线模式".into())
        })?;

    let ip = read_wifi_ip(&adb.path, &usb).ok_or_else(|| {
        AppError::Other(
            "手机没连 Wi-Fi（看起来只走移动数据）：无线调试要求手机与电脑在同一个局域网".into(),
        )
    })?;

    let mut args = base_args(&usb);
    args.push("tcpip".to_string());
    args.push(WIRELESS_PORT.to_string());
    let output = run_process(&adb.path, &args, WIRELESS_TIMEOUT)?;
    if !output.success {
        return Err(AppError::Other(format!(
            "把 adbd 切到无线模式失败：{}",
            output.error_tail()
        )));
    }

    // adbd 是重启式的，端口要通过 getprop 确认真的起来了。
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut port = None;
    while Instant::now() < deadline {
        port = read_tcp_port(&adb.path, &usb);
        if port == Some(WIRELESS_PORT) {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let port = port.filter(|port| *port == WIRELESS_PORT).ok_or_else(|| {
        AppError::Other(
            "adbd 没有进 TCP 模式（可能是系统限制）：改用手机上的「无线调试」+ 配对码".into(),
        )
    })?;

    let address = format!("{ip}:{port}");
    run_connect(&adb.path, &address)?;
    remember_wireless(app, &address);
    log::info!("无线调试已连上：{address}");
    Ok(wireless_info(app))
}

fn is_ipv4(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 4 && parts.iter().all(|part| part.parse::<u8>().is_ok())
}

/// 从 `arp -a` 的输出里挑出邻居 IPv4。
///
/// 热点里的手机**必然**在邻居表里（它要跟网关 DHCP/上网通信），所以这是最靠得住的
/// 「自动找 IP」手段。表头与掩码/广播行要跳过——中文与英文 Windows 的表头不同，
/// 这里按「第一列是 IPv4、第二列像 MAC」来认，不依赖表头文案。
fn parse_arp_neighbors(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(ip) = tokens.next() else { continue };
        if !is_ipv4(ip) || ip.starts_with("255.") || ip.starts_with("224.") {
            continue;
        }
        let Some(mac) = tokens.next() else { continue };
        let mac_parts: Vec<&str> = mac.split(['-', ':']).collect();
        if mac_parts.len() != 6 {
            continue;
        }
        // 广播地址（ff-ff-ff-ff-ff-ff）与组播 MAC 不是设备，别拿去试连。
        let lower = mac.to_lowercase();
        if mac_parts.iter().all(|part| part.eq_ignore_ascii_case("ff")) || lower.starts_with("01-")
        {
            continue;
        }
        if !found.iter().any(|item| item == ip) {
            found.push(ip.to_string());
        }
    }
    found
}

/// `adb mdns services` 的输出 → `ip:端口`。
///
/// 手机开着「无线调试」时 adbd 会自己广播 `_adb-tls-connect._tcp`，末段就是地址与端口；
/// `adb tcpip 5555` 模式通常不广播，所以这条只是**先手**，后面还有邻居表兜底。
fn parse_mdns_services(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some(last) = line.split('\t').next_back() else {
            continue;
        };
        let last = last.trim();
        let Some((host, port)) = last.rsplit_once(':') else {
            continue;
        };
        if !is_ipv4(host) || port.parse::<u16>().is_err() {
            continue;
        }
        if !found.iter().any(|item| item == last) {
            found.push(last.to_string());
        }
    }
    found
}

fn list_mdns_services(adb: &Path) -> Vec<String> {
    let args = vec!["mdns".to_string(), "services".to_string()];
    match run_process(adb, &args, Duration::from_secs(4)) {
        Ok(output) => parse_mdns_services(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

fn arp_neighbors() -> Vec<String> {
    match run_process(
        Path::new("arp"),
        &["-a".to_string()],
        Duration::from_secs(4),
    ) {
        Ok(output) => parse_arp_neighbors(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

/// 快速试一次连接：超时短、失败就是失败（发现流程要逐个试，不能每个都等 12 秒）。
fn try_connect_fast(adb: &Path, address: &str) -> bool {
    let args = vec!["connect".to_string(), address.to_string()];
    let Ok(output) = run_process(adb, &args, Duration::from_secs(3)) else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.success || text.contains("unable") || text.contains("failed") {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if list_devices(adb)
            .iter()
            .any(|device| device.serial == address && device.authorized)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// 自动发现并连上手机：记住的地址 → mDNS 广播 → 局域网邻居表。
///
/// 这条路径的意义是「不用手填 IP」：电脑开热点、手机连上之后，它的地址一定在
/// `arp -a` 里；而 `adb tcpip 5555` 已经让它在这个端口上监听。
pub fn wireless_discover(app: &AppHandle) -> AppResult<WirelessInfo> {
    let adb = require_adb(app)?;
    let info = wireless_info(app);
    if info.connected {
        return Ok(info);
    }

    let mut tried: Vec<String> = Vec::new();

    // 1) 上次连过的地址最快。
    let remembered = config_snapshot(app).phone_wireless_addr;
    if !remembered.is_empty() {
        tried.push(remembered.clone());
        if try_connect_fast(&adb.path, &remembered) {
            remember_wireless(app, &remembered);
            log::info!("无线调试（记住的地址）已连上：{remembered}");
            return Ok(wireless_info(app));
        }
    }

    // 2) mDNS：开着「无线调试」的手机自己会广播（端口是随机的，用广播里的）。
    for address in list_mdns_services(&adb.path) {
        if tried.iter().any(|item| item == &address) {
            continue;
        }
        if try_connect_fast(&adb.path, &address) {
            remember_wireless(app, &address);
            log::info!("无线调试（mDNS 发现）已连上：{address}");
            return Ok(wireless_info(app));
        }
        tried.push(address);
    }

    // 3) 邻居表：`adb tcpip 5555` 的端口是固定的，逐个试连。
    for ip in arp_neighbors() {
        let address = format!("{ip}:{WIRELESS_PORT}");
        if tried.iter().any(|item| item == &address) {
            continue;
        }
        if try_connect_fast(&adb.path, &address) {
            remember_wireless(app, &address);
            log::info!("无线调试（局域网邻居）已连上：{address}");
            return Ok(wireless_info(app));
        }
        tried.push(address);
    }

    Err(AppError::Other(format!(
        "没在局域网里找到可连接的手机（试过 {} 个地址）：先确认手机连的是电脑热点，并且用数据线点过一次「一键切到无线调试」（它会把 adbd 切到 5555）。",
        tried.len()
    )))
}

/// 手动连接一个已知地址（手机换网络、或上次连过的地址）。
pub fn wireless_connect(app: &AppHandle, address: &str) -> AppResult<WirelessInfo> {
    let adb = require_adb(app)?;
    let address = checked_address(address)?;
    run_connect(&adb.path, &address)?;
    remember_wireless(app, &address);
    log::info!("无线调试已连上：{address}");
    Ok(wireless_info(app))
}

/// 断开无线连接（USB 那条不受影响）。记住的地址保留，方便再点「连接」。
pub fn wireless_disconnect(app: &AppHandle, address: &str) -> AppResult<WirelessInfo> {
    let adb = require_adb(app)?;
    let address = checked_address(address)?;
    let args = vec!["disconnect".to_string(), address.clone()];
    let output = run_process(&adb.path, &args, WIRELESS_TIMEOUT)?;
    if !output.success {
        return Err(AppError::Other(format!(
            "断开 {address} 失败：{}",
            output.error_tail()
        )));
    }

    // 当前设备别再指着这个已经断掉的串。
    let state = app.state::<config::AppState>();
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.phone_serial == address {
        guard.phone_serial.clear();
    }
    let snapshot = guard.clone();
    drop(guard);
    if let Err(err) = config::save(app, &snapshot) {
        log::warn!("保存断连状态失败：{err}");
    }
    log::info!("已断开无线调试：{address}");
    Ok(wireless_info(app))
}

/// Android 11+ 的「无线调试」配对（手机端会给一个随机端口 + 6 位配对码）。
///
/// 只有 `adb tcpip` 走不通的机型才需要它——配对成功后再 `wireless_connect`。
pub fn wireless_pair(app: &AppHandle, address: &str, code: &str) -> AppResult<String> {
    let adb = require_adb(app)?;
    let address = checked_address(address)?;
    let code = code.trim();
    if code.is_empty() || code.chars().count() > 12 || code.chars().any(char::is_whitespace) {
        return Err(AppError::Other("配对码要填手机屏幕上那 6 位数字".into()));
    }
    let args = vec!["pair".to_string(), address.clone(), code.to_string()];
    let output = run_process(&adb.path, &args, WIRELESS_TIMEOUT)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = output.error_tail();
    if !output.success || stdout.contains("Failed") || stderr.contains("Failed") {
        return Err(AppError::Other(format!(
            "配对失败：{}",
            if stdout.is_empty() { stderr } else { stdout }
        )));
    }
    log::info!("无线调试配对成功：{address}");
    Ok(if stdout.is_empty() {
        "配对成功".to_string()
    } else {
        stdout
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_realistic_devices_output() {
        let text = "\
List of devices attached
d989934a               device usb:1-1 product:marble model:redmi_k90_pro_max device:marble transport_id:1
127.0.0.1:5555         offline
emulator-5554          unauthorized
";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 3);

        assert_eq!(devices[0].serial, "d989934a");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[0].model, "redmi_k90_pro_max");
        assert_eq!(devices[0].product, "marble");
        assert!(devices[0].authorized);

        assert_eq!(devices[1].state, "offline");
        assert!(!devices[1].authorized);
        assert!(devices[1].model.is_empty());

        assert_eq!(devices[2].state, "unauthorized");
        assert!(!devices[2].authorized);
    }

    #[test]
    fn parses_no_permissions_state_without_losing_words() {
        let devices = parse_devices(
            "List of devices attached\nabc123\tno permissions (missing udev rules?)\n",
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "abc123");
        assert!(devices[0].state.starts_with("no permissions"));
        assert!(!devices[0].authorized);
    }

    #[test]
    fn ignores_empty_and_daemon_noise() {
        let text = "\n* daemon not running; starting now at tcp:5037\nList of devices attached\n\n";
        assert!(parse_devices(text).is_empty());
    }

    #[test]
    fn png_passthrough_and_crlf_repair() {
        let mut png = PNG_MAGIC.to_vec();
        png.extend([0x00, 0x01]);
        assert_eq!(normalize_png(png.clone()).unwrap(), png);

        // `\n` → `\r\n` 之后头变成 89 50 4e 47 0d 0d 0a 1a 0a，修复后应还原。
        let mut broken = Vec::new();
        for byte in &png {
            if *byte == b'\n' {
                broken.push(b'\r');
            }
            broken.push(*byte);
        }
        assert_eq!(normalize_png(broken).unwrap(), png);
    }

    #[test]
    fn non_png_payload_is_an_error_not_a_blank_image() {
        assert!(normalize_png(b"error: device offline\n".to_vec()).is_err());
    }

    #[test]
    fn serial_must_be_present_and_clean() {
        assert!(checked_serial("   ").is_err());
        assert!(checked_serial("d989934a").is_ok());
        assert!(checked_serial("bad serial").is_err());
    }

    #[test]
    fn keycodes_match_android_constants() {
        assert_eq!(PhoneKey::Back.code(), 4);
        assert_eq!(PhoneKey::Home.code(), 3);
        assert_eq!(PhoneKey::Recents.code(), 187);
        assert_eq!(PhoneKey::Power.code(), 26);
        assert_eq!(PhoneKey::Wake.code(), 224);
        assert_eq!(PhoneKey::Sleep.code(), 223);
        assert_eq!(PhoneKey::VolumeUp.code(), 24);
        assert_eq!(PhoneKey::VolumeDown.code(), 25);
    }

    #[test]
    fn parses_wlan_ip_from_ip_addr_output() {
        let text = "\
3: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP
    inet 192.168.137.232/24 brd 192.168.137.255 scope global wlan0
       valid_lft forever preferred_lft forever
    inet6 fe80::1234:5678/64 scope link
";
        assert_eq!(parse_wifi_ip(text), Some("192.168.137.232".to_string()));
        // 只有 IPv6 / 空输出都不能当成本地地址（没连 Wi-Fi 时就是这样）。
        assert_eq!(parse_wifi_ip("    inet6 fe80::1/64 scope link"), None);
        assert_eq!(parse_wifi_ip(""), None);
    }

    #[test]
    fn tcp_port_only_accepts_real_ports() {
        assert_eq!(parse_tcp_port("5555"), Some(5555));
        assert_eq!(parse_tcp_port(" 5555\n"), Some(5555));
        // 0 是 adbd 回到 USB 模式后的值，等于「没监听」。
        assert_eq!(parse_tcp_port("0"), None);
        assert_eq!(parse_tcp_port(""), None);
        assert_eq!(parse_tcp_port("abc"), None);
    }

    #[test]
    fn wireless_info_derives_ip_and_port_from_tcp_device() {
        // 拔了线之后 `adb devices` 里只剩 TCP 那条：IP / 端口必须从连接地址推出来，
        // 否则界面会自相矛盾（明明连着 Wi-Fi，却显示「手机 IP：无」）。
        let devices = parse_devices(
            "List of devices attached\n192.168.137.232:5555   device product:myron model:25102RKBEC\n",
        );
        let info = build_wireless_info(Path::new("adb-not-installed"), &devices, "");
        assert!(info.connected);
        assert!(info.usb_serial.is_none());
        assert_eq!(info.ip.as_deref(), Some("192.168.137.232"));
        assert_eq!(info.tcp_port, Some(5555));
        assert_eq!(info.address.as_deref(), Some("192.168.137.232:5555"));
    }

    #[test]
    fn wireless_info_falls_back_to_remembered_address() {
        let info = build_wireless_info(Path::new("adb-not-installed"), &[], "192.168.137.7:5555");
        assert!(!info.connected);
        assert_eq!(info.address.as_deref(), Some("192.168.137.7:5555"));
    }

    #[test]
    fn parses_arp_neighbors() {
        // 中文与英文 Windows 的表头不同，按「IPv4 + MAC」认行，不依赖文案。
        let zh = "\
接口: 192.168.137.1 --- 0x13
  Internet 地址         物理地址              类型
  192.168.137.255       ff-ff-ff-ff-ff-ff     静态
  224.0.0.22            01-00-5e-00-00-16     静态
  192.168.137.232       a4-7b-9d-11-22-33     动态
  192.168.137.1         aa-bb-cc-dd-ee-ff     动态
";
        assert_eq!(
            parse_arp_neighbors(zh),
            vec!["192.168.137.232", "192.168.137.1"]
        );

        let en = "\
Interface: 192.168.137.1 --- 0x13
  Internet Address      Physical Address      Type
  192.168.137.99        a4-7b-9d-11-22-44     dynamic
";
        assert_eq!(parse_arp_neighbors(en), vec!["192.168.137.99"]);
        assert!(parse_arp_neighbors("").is_empty());
    }

    #[test]
    fn parses_mdns_services() {
        let text = "\
List of discovered mdns services
adb-25102RKBEC-abc123	_adb-tls-connect._tcp	192.168.137.232:41235
adb-25102RKBEC-def456	_adb-tls-pairing._tcp	192.168.137.232:39111
";
        assert_eq!(
            parse_mdns_services(text),
            vec!["192.168.137.232:41235", "192.168.137.232:39111"]
        );
        assert!(parse_mdns_services("List of discovered mdns services").is_empty());
    }

    #[test]
    fn connect_address_is_validated() {
        assert_eq!(
            checked_address(" 192.168.137.232:5555 ").unwrap(),
            "192.168.137.232:5555"
        );
        // 少了端口、端口为 0、带空白的多余参数都必须挡掉。
        assert!(checked_address("192.168.137.232").is_err());
        assert!(checked_address("192.168.137.232:0").is_err());
        assert!(checked_address(" :5555").is_err());
        assert!(checked_address("192.168.1.2:5555 extra").is_err());
    }
}
