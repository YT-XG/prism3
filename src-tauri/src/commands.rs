use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_opener::OpenerExt;

use crate::ble::{
    self, AncGradient, BatteryReading, BleCandidate, BleState, BleStateSnapshot, EqPreset,
    NoiseMode,
};
use crate::clipboard::{self, ClipEntryMeta, ClipKindFilter, ClipState, ClipboardState};
use crate::config::{AppConfig, AppState, DockEdge};
use crate::error::{AppError, AppResult};
use crate::file_index::{self, FileIndexState, IndexState, SearchHit};
use crate::paste::{self, PasteOutcome};
use crate::status::{self, AppStatus, AppStatusState};
use crate::updater::{self, UpdateSnapshot};

/// 快捷小窗**灰条**窗口标签。灰条独占交互与状态。
const QUICK_PANEL_LABEL: &str = "quick-panel";
/// 快捷小窗**面板**窗口标签。高度恒定、宽度随页面变化，仅由灰条窗口驱动。
const QUICK_PANEL_BODY_LABEL: &str = "quick-panel-body";
/// 通知灰条窗口开关/边缘/偏移变化的事件名。
const QUICK_PANEL_EVENT: &str = "quick-panel:state";
/// 通知面板窗口播放展开/收起动画的事件名。
const QUICK_PANEL_BODY_EVENT: &str = "quick-panel:body-state";
/// 广播应用运行状态变化的事件名（灰条据此换色相与动画）。广播给所有窗口：状态是应用级的。
const APP_STATUS_EVENT: &str = "app:status";

/// 面板状态事件载荷。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuickPanelState {
    enabled: bool,
    edge: DockEdge,
    offset: Option<i32>,
}

/// 面板窗口展开/收起事件载荷；`edge` 决定动画从哪一侧擦出。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuickPanelBodyState {
    open: bool,
    edge: DockEdge,
}

/// 面板窗口句柄。
fn quick_panel_body(app: &AppHandle) -> AppResult<tauri::WebviewWindow> {
    app.get_webview_window(QUICK_PANEL_BODY_LABEL)
        .ok_or_else(|| AppError::Other("快捷小窗面板窗口不存在".into()))
}

/// 灰条窗口尺寸（与前端 `LINE_WIDTH` / `LINE_HEIGHT` 保持一致；前端会在 show 前后再落实一次）。
const QUICK_PANEL_BAR_WIDTH: f64 = 8.0;
const QUICK_PANEL_BAR_HEIGHT: f64 = 64.0;

/// 按需创建灰条窗口 `quick-panel`（透明、置顶、不占任务栏、不可聚焦，仅拖动/换边时改几何）。
fn build_bar_window(app: &AppHandle) -> AppResult<()> {
    WebviewWindowBuilder::new(
        app,
        QUICK_PANEL_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 快捷小窗")
    .inner_size(QUICK_PANEL_BAR_WIDTH, QUICK_PANEL_BAR_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()?;
    Ok(())
}

/// 面板窗口沿交叉轴的固定尺寸：宽 = 最大页宽（帮助页），高 = 面板高度。
/// 与前端 `PANEL_WINDOW_WIDTH` / `PANEL_HEIGHT` 常量保持一致。
///
/// **窗口尺寸恒定、可见期间从不缩放**：WebView 内容跟不上窗口缩放，缩放瞬间
/// 旧帧会贴着新视口左上角被画出一两帧（面板整体平移 / 瘦成一条再弹回）。
/// 换页调宽只改面板盒子自身的 CSS 宽度，窗口几何不动（见 docs/quick-panel-flash-postmortem.md）。
const QUICK_PANEL_BODY_WIDTH: f64 = 520.0;
const QUICK_PANEL_BODY_HEIGHT: f64 = 520.0;

/// 按需创建面板窗口 `quick-panel-body`（恒为最大页宽 × 面板高度，可见期间不缩放）。
fn build_body_window(app: &AppHandle) -> AppResult<()> {
    WebviewWindowBuilder::new(
        app,
        QUICK_PANEL_BODY_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 快捷小窗面板")
    .inner_size(QUICK_PANEL_BODY_WIDTH, QUICK_PANEL_BODY_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()?;
    Ok(())
}

/// 确保两个快捷小窗窗口已创建（幂等）。启用快捷小窗时才创建，避免功能关闭时白占 WebView。
pub fn ensure_quick_panel_windows(app: &AppHandle) -> AppResult<()> {
    if app.get_webview_window(QUICK_PANEL_LABEL).is_none() {
        build_bar_window(app)?;
    }
    if app.get_webview_window(QUICK_PANEL_BODY_LABEL).is_none() {
        build_body_window(app)?;
    }
    Ok(())
}

/// 销毁两个快捷小窗窗口（不存在时忽略）。禁用快捷小窗时释放其 WebView 内存。
pub fn destroy_quick_panel_windows(app: &AppHandle) {
    // 窗口即将消失，记录的「原前台窗口」不再有意义。
    crate::focus::forget();
    for label in [QUICK_PANEL_BODY_LABEL, QUICK_PANEL_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            if let Err(err) = window.destroy() {
                log::warn!("销毁快捷小窗窗口 {label} 失败：{err}");
            }
        }
    }
}

/// 依配置同步快捷小窗窗口的存亡：启用则确保存在并广播状态，禁用则销毁。
///
/// 刚创建的窗口其前端尚未挂载、收不到状态事件，但挂载后会主动 `get_config` 兜底。
pub fn sync_quick_panel_windows(app: &AppHandle, config: &AppConfig) {
    if config.quick_panel {
        match ensure_quick_panel_windows(app) {
            Ok(()) => emit_quick_panel_state(app, config),
            Err(err) => log::error!("创建快捷小窗窗口失败：{err}"),
        }
    } else {
        destroy_quick_panel_windows(app);
    }
}

/// 依配置同步**文件索引**的存亡。
///
/// 启用则起一轮重建；禁用则取消在途扫描——**落盘索引保留**，因此再次开启时能秒开。
pub fn sync_file_index(app: &AppHandle, config: &AppConfig) {
    let state = app.state::<FileIndexState>();
    state.set_enabled(config.file_search);

    if !config.file_search {
        state.broadcast(app);
        return;
    }
    start_file_index_build(app, &config.file_search_roots);
}

/// 在独立线程里重建索引：命令立即返回，进度通过 `search:index-state` 广播。
///
/// 重复调用会作废上一轮：旧扫描按代号自行放弃（见 `file_index.rs` 的 `is_stale`）。
fn start_file_index_build(app: &AppHandle, configured_roots: &[String]) {
    let app = app.clone();
    let configured_roots = configured_roots.to_vec();
    std::thread::spawn(move || {
        let state = app.state::<FileIndexState>();
        if !state.is_enabled() {
            return;
        }
        let (roots, missing_roots) = file_index::resolve_roots(&app, &configured_roots);
        if let Err(err) = file_index::rebuild_blocking(&app, &state, roots, missing_roots) {
            log::error!("构建文件索引失败：{err}");
            state.fail(format!("构建文件索引失败：{err}"));
            state.broadcast(&app);
        }
    });
}

/// 读取当前配置。
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 读取用户选中的壁纸并编码成 data URL（直接给 CSS `background-image`）。
///
/// 走 Rust 而不是前端 `fs` 插件：系统对话框返回的是任意路径，`fs:default` 的
/// scope 覆盖不到；这里只读一次、按文件头判类型，再交给 WebView。
/// 16 MiB 上限，避免一张超大图把 WebView 内存打满。
#[tauri::command(async)]
pub fn read_wallpaper(path: String) -> AppResult<String> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;

    let file = std::path::PathBuf::from(&path);
    let meta = std::fs::metadata(&file)?;
    if !meta.is_file() {
        return Err(AppError::Other("壁纸路径不是文件".into()));
    }
    if meta.len() > MAX_BYTES {
        return Err(AppError::Other("壁纸图片超过 16 MiB".into()));
    }

    let bytes = std::fs::read(&file)?;
    let mime = wallpaper_mime(&file, &bytes).ok_or_else(|| {
        AppError::Other("不支持的图片格式（png / jpg / webp / bmp / gif）".into())
    })?;
    Ok(format!("data:{mime};base64,{}", base64_encode(&bytes)))
}

/// 先按文件头识别格式，扩展名只作兜底（用户可能把 png 存成 .jpg）。
fn wallpaper_mime(path: &std::path::Path, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    }
}

/// 标准 base64（带 `=` 填充）。只服务壁纸这一条路径，不引入额外 crate。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 0x3F] as char);
        out.push(TABLE[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

/// 保存配置，同时更新内存状态与磁盘文件。
///
/// `quick_panel_edge` 与 `quick_panel_y` 由灰条窗口独占写入，这里以内存态为准，
/// 避免主窗口持有的旧值把面板拖动后的位置覆盖掉。
///
/// **三个 BLE 键同理（见 `ble::preserve_config_keys`）**：主窗口与快捷小窗各持一份
/// 配置副本，整份回写会把另一窗口刚改的 BLE 键用过期副本覆盖掉（lost update）。
/// 因此这里把它们换回内存里的权威值 —— 想改 BLE 键只能走下面四个专用命令。
#[tauri::command]
pub fn set_config(
    app: AppHandle,
    state: State<'_, AppState>,
    mut config: AppConfig,
) -> AppResult<()> {
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    config.quick_panel_edge = guard.quick_panel_edge;
    config.quick_panel_y = guard.quick_panel_y;
    ble::preserve_config_keys(&mut config, &guard);
    let panel_toggled = guard.quick_panel != config.quick_panel;
    let search_toggled = guard.file_search != config.file_search;
    let search_roots_changed = guard.file_search_roots != config.file_search_roots;
    let theme_changed = guard.theme != config.theme;
    let auto_start_changed = guard.auto_start != config.auto_start;
    let clipboard_toggled = guard.clipboard_history != config.clipboard_history;
    let clipboard_settings_changed = guard.clipboard_retention_days
        != config.clipboard_retention_days
        || guard.clipboard_capture_images != config.clipboard_capture_images
        || guard.clipboard_paste_on_click != config.clipboard_paste_on_click;

    *guard = config.clone();
    drop(guard);

    crate::config::save(&app, &config)?;

    if panel_toggled {
        sync_quick_panel_windows(&app, &config);
    }
    if theme_changed {
        crate::sync_window_background(&app, config.theme);
    }
    if auto_start_changed {
        crate::autostart::sync(&app, &config);
    }
    if search_toggled || search_roots_changed {
        sync_file_index(&app, &config);
    }
    if clipboard_toggled || clipboard_settings_changed {
        clipboard::sync(&app, &config);
    }
    // BLE 的三个键上面已经被换回权威值（这里必然「无变化」），调用它是为了让
    // 「配置改了 → 运行时跟着改」只有一条共享路径（见 `ble::sync_from_config`）。
    ble::sync_from_config(&app);
    Ok(())
}

/// 恢复默认配置并落盘，返回恢复后的配置。
#[tauri::command]
pub fn reset_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppConfig> {
    let default = AppConfig::default();

    *state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = default.clone();

    crate::config::save(&app, &default)?;

    sync_quick_panel_windows(&app, &default);
    crate::sync_window_background(&app, default.theme);
    crate::autostart::sync(&app, &default);
    sync_file_index(&app, &default);
    clipboard::sync(&app, &default);
    // 恢复默认是**显式**动作（不是过期副本回写）：BLE 键也要跟着回默认并停掉看门狗。
    ble::sync_from_config(&app);
    Ok(default)
}

/// 只改 BLE 相关字段：在写锁内读-改-写，随后落盘。
///
/// 与 `set_quick_panel_dock` 同一手法：**只碰自己那一项**，绝不整份回写，
/// 因此两个窗口各自的配置副本不会互相覆盖（lost update 的根治办法）。
fn update_ble_config<F>(app: &AppHandle, state: &AppState, mutate: F) -> AppResult<()>
where
    F: FnOnce(&mut AppConfig),
{
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    mutate(&mut guard);
    let config = guard.clone();
    drop(guard);

    crate::config::save(app, &config)
}

/// 开关自动连接（写配置 + 立即生效：起停看门狗）。
///
/// 关掉时**不动已有链路**（用户可能正戴着听歌）；打开时若已知目标就起看门狗。
#[tauri::command(async)]
pub fn set_ble_auto_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    update_ble_config(&app, &state, |config| config.ble_auto_connect = enabled)?;
    ble::sync_from_config(&app);
    Ok(())
}

/// 指定自动连接的目标设备（写配置 + 立即生效）。
///
/// **同一地址是空操作**：配置本来就该是这个值，而重来一遍会白白闪断一次链路
/// （前端的「连接 / 切换」按钮与「加入列表后设为当前」都会重复下发同一个地址）。
/// 地址不同才停掉当前会话并按需改盯新目标（见 `ble::sync_from_config`）。
#[tauri::command(async)]
pub fn set_ble_active_device(
    app: AppHandle,
    state: State<'_, AppState>,
    address: String,
) -> AppResult<()> {
    let unchanged = {
        let guard = state
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.ble_active_address.as_deref() == Some(address.as_str())
    };
    if unchanged {
        return Ok(());
    }

    update_ble_config(&app, &state, |config| {
        config.ble_active_address = Some(address)
    })?;
    ble::sync_from_config(&app);
    Ok(())
}

/// 把一台设备加进已知列表（按地址去重；**不覆盖已有的非空缓存名**）。
///
/// 本身不改会话：要不要连它由 `set_ble_active_device` / `connect_earbuds` 决定。
#[tauri::command(async)]
pub fn add_ble_device(
    app: AppHandle,
    state: State<'_, AppState>,
    address: String,
    name: Option<String>,
) -> AppResult<()> {
    // 空串 / 全空白视为「没给名字」：绝不能把缓存里已有的名字抹成空。
    let name = name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    update_ble_config(&app, &state, |config| {
        let entry = ble::upsert_device(config, &address);
        if let Some(name) = name {
            entry.name = Some(name);
        }
    })?;
    // 已知列表变了：刷快照（扫描结果里的 `known` 与设备名都据此显示）。
    ble::sync_from_config(&app);
    Ok(())
}

/// 把一台设备移出已知列表。
///
/// - 被移出的正好是**当前目标**时一并清掉活动地址：否则配置里会留一个「用户刚删掉、
///   自动连接却还在追」的悬空目标；
/// - 正在连接 / 已连接的就是这台设备时先结束会话：会话线程负责 `disconnect`，
///   保住「耳机一次只接受一个 LE 中央设备」这条不变量。
#[tauri::command(async)]
pub fn remove_ble_device(
    app: AppHandle,
    state: State<'_, AppState>,
    address: String,
) -> AppResult<()> {
    if app.state::<BleState>().owns_address(&address) {
        ble::stop_session(&app);
    }

    update_ble_config(&app, &state, |config| {
        config
            .ble_devices
            .retain(|entry| !entry.address.eq_ignore_ascii_case(&address));
        if config
            .ble_active_address
            .as_deref()
            .is_some_and(|active| active.eq_ignore_ascii_case(&address))
        {
            config.ble_active_address = None;
        }
    })?;
    ble::sync_from_config(&app);
    Ok(())
}

/// 更新快捷小窗吸附的边缘与沿边偏移（由灰条窗口在拖动结束时调用）。
#[tauri::command]
pub fn set_quick_panel_dock(
    app: AppHandle,
    state: State<'_, AppState>,
    edge: DockEdge,
    offset: Option<i32>,
) -> AppResult<()> {
    let mut guard = state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.quick_panel_edge = edge;
    guard.quick_panel_y = offset;
    let config = guard.clone();
    drop(guard);

    crate::config::save(&app, &config)
}

/// 一次性落实窗口的位置与尺寸：**单次 `SetWindowPos`**，位置与尺寸同帧生效。
///
/// 不能用 Tauri 的 `set_position` + `set_size`：那是两次 Win32 调用，两次之间窗口
/// 处于「新位置 + 旧尺寸」的中间态。面板内容贴灰条一侧锚定，中间态会让整块内容
/// 平移一个宽度差（帮助页 ↔ 其它页相差 160 逻辑像素；右侧吸附时表现为面板被推到
/// 屏幕外、露出一两帧错位），灰条换边时同样可能瞬时越出工作区。
#[cfg(windows)]
fn apply_window_bounds(
    window: &tauri::WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> AppResult<()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    // 逻辑像素 → 物理像素：与 Tauri 的 LogicalPosition / LogicalSize 同一换算。
    let scale = window.scale_factor()?;
    let (px, py) = ((x * scale).round() as i32, (y * scale).round() as i32);
    let (pw, ph) = (
        (width * scale).round() as i32,
        (height * scale).round() as i32,
    );

    // SAFETY: hwnd 属于本进程持有的窗口；SetWindowPos 在当前线程同步改窗口几何。
    let outcome = unsafe {
        SetWindowPos(
            HWND(window.hwnd()?.0),
            None,
            px,
            py,
            pw,
            ph,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
        )
    };
    outcome.map_err(|cause| AppError::Other(format!("设置窗口几何失败：{cause}")))
}

/// 非 Windows 退化为「先位置后尺寸」两次调用（本项目只发布 Windows，仅为可编译性保留）。
#[cfg(not(windows))]
fn apply_window_bounds(
    window: &tauri::WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> AppResult<()> {
    window.set_position(LogicalPosition::new(x, y))?;
    window.set_size(tauri::LogicalSize::new(width, height))?;
    Ok(())
}

/// 一次调用同时设置**灰条窗口**的尺寸与位置（拖动每帧调用，合并两次 IPC 为一次）。
///
/// 展开/收起面板时不会调用它：灰条窗口的几何只在拖动与换边时变化，
/// 因此「灰条仅在被拖动时移动」是结构上成立的，而不是靠逐帧补偿动画。
#[tauri::command]
pub fn set_quick_panel_geometry(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> AppResult<()> {
    let window = app
        .get_webview_window(QUICK_PANEL_LABEL)
        .ok_or_else(|| AppError::Other("快捷小窗窗口不存在".into()))?;

    // 位置与尺寸一次落定：分两次改会让窗口瞬时越出工作区（多显示器下溢到相邻副屏）。
    apply_window_bounds(&window, x, y, width, height)
}

/// 仅移动**面板窗口**：拖动灰条时逐帧跟随；面板窗口尺寸恒定（最大页宽 × 面板高度），
/// 任何路径都不再缩放它（可见期间缩放会残留旧帧闪动，见 `build_body_window` 注释）。
#[tauri::command]
pub fn set_quick_panel_body_position(app: AppHandle, x: f64, y: f64) -> AppResult<()> {
    quick_panel_body(&app)?.set_position(LogicalPosition::new(x, y))?;
    Ok(())
}

/// 展开面板：定位（只移动，尺寸恒定）→ 显示 → 通知面板窗口播放展开动画。
///
/// `width`/`height` 恒为 `QUICK_PANEL_BODY_WIDTH/HEIGHT`（前端传同一常量），
/// 这里仍一并落实：对已存在窗口是无操作缩放（不触发内容重排），但可自愈
/// 尺寸漂移。真正的宽度变化由面板盒子自身的 CSS 过渡承担，窗口不缩放。
#[tauri::command]
pub fn open_quick_panel_body(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    edge: DockEdge,
) -> AppResult<()> {
    let window = quick_panel_body(&app)?;
    // show 前后各落实一次定位，避免隐藏期间窗口被移动后残留旧位置。
    apply_window_bounds(&window, x, y, width, height)?;
    window.show()?;
    apply_window_bounds(&window, x, y, width, height)?;
    emit_quick_panel_body_state(&app, true, edge)?;
    Ok(())
}

/// 收起面板：只通知面板窗口播放收起动画，动画结束后由面板窗口自行隐藏。
#[tauri::command]
pub fn close_quick_panel_body(app: AppHandle, edge: DockEdge) -> AppResult<()> {
    emit_quick_panel_body_state(&app, false, edge)
}

/// 隐藏面板窗口（面板窗口在收起动画结束后调用；灰条窗口在禁用兜底时也会调用）。
///
/// 顺带退出「输入聚焦」模式：**面板不可见 ⇒ 一定不可聚焦**，这条不变量由本命令兜底，
/// 而不是依赖前端每次都记得先调 `blur_quick_panel_body`。
#[tauri::command]
pub fn hide_quick_panel_body(app: AppHandle) -> AppResult<()> {
    if let Err(err) = crate::focus::release(&app) {
        log::warn!("收回快捷小窗面板的聚焦模式失败：{err}");
    }
    quick_panel_body(&app)?.hide()?;
    Ok(())
}

/// 让面板窗口进入「输入聚焦」模式（点击搜索框时调用）。返回是否真的聚焦成功。
///
/// 面板平时不可聚焦（悬停划过不抢焦点）；WebView 的键盘输入需要宿主窗口持有焦点，
/// 因此只有真正要打字时才临时切换，并在退出时把焦点还给用户原来的应用（见 `focus.rs`）。
#[tauri::command(async)]
pub fn focus_quick_panel_body(app: AppHandle) -> AppResult<bool> {
    crate::focus::promote(&app)
}

/// 退出「输入聚焦」模式并把焦点还给进入前的系统前台窗口（Esc / 收起 / 失焦时调用）。
#[tauri::command]
pub fn blur_quick_panel_body(app: AppHandle) -> AppResult<()> {
    crate::focus::release(&app)
}

/// 查询文件名索引。空关键词返回「最近修改的文件」。
///
/// 十万级条目的线性扫描在毫秒级，但仍然走 worker 线程（`async`）以免占住主线程。
#[tauri::command(async)]
pub fn search_files(
    state: State<'_, FileIndexState>,
    query: String,
    limit: Option<u32>,
) -> Vec<SearchHit> {
    state.query(&query, limit)
}

/// 读取文件索引状态（窗口挂载时兜底拉取：事件不缓存）。
#[tauri::command]
pub fn get_file_index_state(state: State<'_, FileIndexState>) -> IndexState {
    state.snapshot()
}

/// 重建文件索引：立即返回，进度由 `search:index-state` 广播。
#[tauri::command]
pub fn rebuild_file_index(app: AppHandle, config: State<'_, AppState>) -> AppResult<()> {
    let (enabled, roots) = {
        let guard = config
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (guard.file_search, guard.file_search_roots.clone())
    };
    if !enabled {
        return Err(AppError::Other("文件搜索未启用".into()));
    }
    start_file_index_build(&app, &roots);
    Ok(())
}

/// 取消在途扫描（已建好的索引保留可用）。
#[tauri::command]
pub fn cancel_file_index(app: AppHandle, state: State<'_, FileIndexState>) {
    state.cancel();
    state.broadcast(&app);
}

/// 「打开搜索结果」的动作。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenAction {
    /// 用默认程序打开（目录则打开资源管理器）。
    Open,
    /// 在资源管理器中定位并选中该项。
    Reveal,
}

/// 打开 / 定位一条搜索结果。
///
/// 走 Rust 侧 opener（前端没有 opener 的路径权限），并额外校验目标位于索引根之下：
/// 即使前端被注入，也无法借这个命令打开索引范围之外的任意路径。
#[tauri::command]
pub fn open_search_result(
    app: AppHandle,
    state: State<'_, FileIndexState>,
    path: String,
    action: OpenAction,
) -> AppResult<()> {
    let target = std::path::PathBuf::from(&path);
    if !file_index::path_within_roots(&target, &state.roots()) {
        return Err(AppError::Other("目标不在索引范围内或已不存在".into()));
    }

    let opener = app.opener();
    let opened = match action {
        OpenAction::Open => opener.open_path(path, None::<&str>),
        OpenAction::Reveal => opener.reveal_item_in_dir(&target),
    };
    opened.map_err(|err| AppError::Other(format!("打开失败：{err}")))
}

/// 读取系统监控动态快照。
///
/// 用 `async` 属性让采集在 worker 线程执行；同步命令会跑在主线程并阻塞 UI。
#[tauri::command(async)]
pub fn get_system_snapshot(
    state: State<'_, crate::monitor::MonitorState>,
) -> crate::monitor::SystemSnapshot {
    state.snapshot()
}

/// 读取系统静态信息（硬件型号 / OS / 主机名），前端只需取一次并缓存。
#[tauri::command]
pub fn get_system_statics(
    state: State<'_, crate::monitor::MonitorState>,
) -> crate::monitor::SystemStatics {
    state.statics()
}

/// 读取当前运行状态（窗口挂载 / 热重载后兜底拉取：事件不缓存）。
#[tauri::command]
pub fn get_app_status(state: State<'_, AppStatusState>) -> AppStatus {
    state.current()
}

/// 采样一次系统负载并更新运行状态，**仅在档位变化时**广播。
///
/// 由灰条窗口按固定间隔驱动，且只在灯可见时调用，因此不新增常驻采样——
/// 沿用 `MonitorState` 既有的「按需采集」模型。
#[tauri::command(async)]
pub fn refresh_app_status(
    app: AppHandle,
    monitor: State<'_, crate::monitor::MonitorState>,
    status: State<'_, AppStatusState>,
) -> AppStatus {
    let peak = status::peak_percent(&monitor.snapshot());
    let next = status::classify(peak, status.current());

    if status.update(next) {
        // 广播失败不该让命令失败：灯停在旧档位上仍可正常使用。
        if let Err(err) = app.emit(APP_STATUS_EVENT, next) {
            log::warn!("广播运行状态失败：{err}");
        }
    }
    next
}

/// 读取蓝牙耳机会话状态（窗口挂载时兜底拉取：事件不缓存）。
#[tauri::command(async)]
pub fn get_ble_state(state: State<'_, BleState>) -> AppResult<BleStateSnapshot> {
    Ok(state.snapshot())
}

/// 连接蓝牙耳机：启动会话线程（扫描 → 连接 → 订阅 → 收帧）。已在会话中时空操作。
///
/// **连接必须是用户显式触发的**（或由看门狗扫到目标后触发）：耳机一次只接受一个
/// LE 中央设备，官方 App 可能正占着链路，因此失败只作为普通错误上报，不重试、不 panic。
#[tauri::command(async)]
pub fn connect_earbuds(app: AppHandle) -> AppResult<()> {
    ble::connect_earbuds(&app)
}

/// 断开蓝牙耳机并结束会话（幂等）；状态立刻落回 `idle`。
///
/// 同时抑制自动连接，否则看门狗会在几秒内把链路重新建起来（用户就永远断不开）。
#[tauri::command(async)]
pub fn disconnect_earbuds(app: AppHandle) -> AppResult<()> {
    ble::disconnect_earbuds(&app);
    Ok(())
}

/// 扫描附近的候选耳机设备。
///
/// 只返回厂商数据命中公司标识 `0x5452` 的设备（耳机不广播名字、不广播服务 UUID，
/// 不过滤就是一屏无法辨认的 MAC 地址）。适配器不可用是错误，**空列表是正常结果**。
#[tauri::command(async)]
pub async fn scan_ble_devices(app: AppHandle) -> AppResult<Vec<BleCandidate>> {
    let known = app.state::<BleState>().known_devices();
    ble::scan_candidates(&known).await
}

/// 读一遍耳机的全部设置项（电量 / EQ / 噪声模式 / 游戏模式 / 降噪梯度 / 双设备）。
#[tauri::command(async)]
pub async fn refresh_earbud_state(app: AppHandle) -> AppResult<()> {
    ble::send_frames(&app, &ble::refresh_requests()).await
}

/// 读一次电量并等它回来（应答超时由会话线程兜底，不会永久挂起）。
#[tauri::command(async)]
pub async fn read_earbud_battery(app: AppHandle) -> AppResult<BatteryReading> {
    ble::read_battery(&app).await
}

/// 切换噪声模式（降噪 / 正常 / 风噪 / 通透）。
#[tauri::command(async)]
pub async fn set_noise_mode(app: AppHandle, mode: NoiseMode) -> AppResult<()> {
    ble::send_frames(&app, &[ble::noise_mode_request(mode)]).await
}

/// 切换降噪梯度。
///
/// 写梯度会让耳机**隐含切回「降噪」模式**，所以这里发出的是两帧：梯度 + 当前模式，
/// 否则用户选的模式会被静默覆盖（规则与注释见 `ble::anc_gradient_frames`）。
#[tauri::command(async)]
pub async fn set_anc_gradient(app: AppHandle, level: AncGradient) -> AppResult<()> {
    let current = app.state::<BleState>().snapshot().noise_mode;
    ble::send_frames(&app, &ble::anc_gradient_frames(level, current)).await
}

/// 开关自适应降噪。
///
/// 这个开关是**本地记忆**（设备从不上报这一位，见 `ble::adaptive_anc_request`）：
/// 因此它比别的设置多一步「先记住，再发帧」——
/// 1. 把值写进当前设备的配置条目（没有条目就建）并落盘；
/// 2. 把厂商那帧发出去（与厂商 App 逐字节相同）；
/// 3. 刷新快照并广播（`adaptiveAnc` 取自配置镜像，所以顺序上必须在写配置之后）。
///
/// 第 2 步即使失败也**照样广播**：本地记忆已经是用户的选择，界面不该停在与配置不一致的
/// 旧值上；发帧的错误仍然抛给前端（`toast`），由用户决定要不要重连再试。
#[tauri::command(async)]
pub async fn set_adaptive_anc(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    let Some(address) = ble::current_device_address(&app) else {
        return Err(AppError::Other("蓝牙未连接（请先连接耳机）".into()));
    };

    update_ble_config(&app, &state, |config| {
        ble::remember_adaptive_anc(config, &address, enabled);
    })?;

    let sent = ble::send_frames(&app, &[ble::adaptive_anc_request(enabled)]).await;
    ble::sync_from_config(&app);
    sent
}

/// 切换 EQ 音效预设。
#[tauri::command(async)]
pub async fn set_eq_preset(app: AppHandle, preset: EqPreset) -> AppResult<()> {
    ble::send_frames(&app, &[ble::eq_preset_request(preset)]).await
}

/// 开关游戏模式。
#[tauri::command(async)]
pub async fn set_game_mode(app: AppHandle, enabled: bool) -> AppResult<()> {
    ble::send_frames(&app, &[ble::game_mode_request(enabled)]).await
}

/// 开关双设备连接。
#[tauri::command(async)]
pub async fn set_dual_device(app: AppHandle, enabled: bool) -> AppResult<()> {
    ble::send_frames(&app, &[ble::dual_device_request(enabled)]).await
}

// ------------------------------ 剪贴板历史 ------------------------------

/// 读取剪贴板历史状态（窗口挂载时兜底拉取：事件不缓存）。
#[tauri::command]
pub fn get_clipboard_state(state: State<'_, ClipboardState>) -> ClipState {
    state.snapshot()
}

/// 查询剪贴板历史。空关键词返回「最近入库」的条目。
///
/// 走 worker 线程（`async`）：历史虽小，但图片条目多时线性扫描 + 预览拼接也不该压在主线程上。
#[tauri::command(async)]
pub fn search_clipboard(
    state: State<'_, ClipboardState>,
    query: String,
    kind: Option<String>,
    limit: Option<usize>,
) -> Vec<ClipEntryMeta> {
    state.query(&query, ClipKindFilter::parse(kind.as_deref()), limit)
}

/// 取一条文本记录的完整正文（列表只拿预览，编辑 / 展开时才按需拉全文）。
#[tauri::command(async)]
pub fn get_clipboard_text(state: State<'_, ClipboardState>, id: u64) -> AppResult<String> {
    state.text_of(id)
}

/// 取一条图片记录，返回 data URL（`thumb` 为真时给列表用的缩略图）。
///
/// 图片一律经命令回传，**不给前端开 asset protocol / 文件路径权限**——
/// 与「打开搜索结果走 Rust 侧 opener」同一考虑。
#[tauri::command(async)]
pub fn get_clipboard_image(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
    thumb: Option<bool>,
) -> AppResult<String> {
    state.image_data_url(&app, id, thumb.unwrap_or(true))
}

/// 把一条记录写进系统剪贴板（不粘贴）。
#[tauri::command(async)]
pub fn copy_clipboard_entry(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
) -> AppResult<()> {
    let content = state.content(&app, id)?;
    let seq = paste::copy(&content)?;
    state.note_self_write(seq);
    Ok(())
}

/// 写进剪贴板并把焦点还给上一个窗口后粘贴（配置关闭自动粘贴时只复制）。
///
/// 返回是否真的粘上了：没粘上时 `reason` 说明原因（前端 `toast`），内容已经在剪贴板里，
/// 用户可以自己按 Ctrl+V —— 绝不静默失败（蓝牙「写回执」那条教训）。
#[tauri::command(async)]
pub fn paste_clipboard_entry(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
) -> AppResult<PasteOutcome> {
    let content = state.content(&app, id)?;
    let seq = paste::copy(&content)?;
    state.note_self_write(seq);

    if !state.snapshot().paste_on_click {
        return Ok(PasteOutcome::copied());
    }
    Ok(paste::paste(&app))
}

/// 修改一条文本记录的正文（图片记录不可编辑）。
#[tauri::command(async)]
pub fn update_clipboard_text(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
    text: String,
) -> AppResult<()> {
    state.update_text(&app, id, &text)
}

/// 删除一条记录（图片条目连文件一起删）。**收藏的记录也能这样手动删掉**。
#[tauri::command]
pub fn delete_clipboard_entry(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
) -> AppResult<()> {
    state.delete(&app, id)
}

/// 设置 / 取消一条记录的收藏（收藏的条目不会被保留天数裁剪、也不被「清空历史」删掉）。
#[tauri::command]
pub fn set_clipboard_pinned(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
    pinned: bool,
) -> AppResult<()> {
    state.set_pinned(&app, id, pinned)
}

/// 清空全部剪贴板历史（连同图片文件）。收藏的记录会被保留，只由手动删除移除。
#[tauri::command]
pub fn clear_clipboard_history(app: AppHandle, state: State<'_, ClipboardState>) -> AppResult<()> {
    state.clear(&app)
}

/// 立即按保留策略清理过期条目。
#[tauri::command]
pub fn prune_clipboard_history(app: AppHandle, state: State<'_, ClipboardState>) -> AppResult<()> {
    state.prune(&app)
}

// ------------------------------ 自动更新 ------------------------------

/// 读取更新状态（窗口挂载时兜底拉取：事件不缓存）。
#[tauri::command]
pub fn get_update_state(app: AppHandle) -> UpdateSnapshot {
    updater::snapshot(&app)
}

/// 检查更新：立即返回「当前快照」，真正的网络请求与下载在后台线程里，
/// 结果经 `update:state` 事件广播（与 `ble:state` / `clipboard:state` 同一模式）。
#[tauri::command]
pub fn check_update(app: AppHandle) -> UpdateSnapshot {
    updater::check(&app, false)
}

/// 重新安装最新版本：跳过版本比较、强制删除已下载的包后重新下载。
#[tauri::command]
pub fn reinstall_latest(app: AppHandle) -> UpdateSnapshot {
    updater::check(&app, true)
}

/// 暂停下载（保留 `.part` 与已下载字节，可 `resume_update` 续传）。
#[tauri::command]
pub fn pause_update(app: AppHandle) -> UpdateSnapshot {
    updater::pause(&app)
}

/// 恢复暂停的下载（Range 续传；服务器不支持 Range 时从头下）。
#[tauri::command]
pub fn resume_update(app: AppHandle) -> UpdateSnapshot {
    updater::resume(&app)
}

/// 取消在途检查/下载：作废本次结果、删除 `.part`，回到可重试状态。
#[tauri::command]
pub fn cancel_update(app: AppHandle) -> UpdateSnapshot {
    updater::cancel(&app)
}

/// 安装已下载的更新并退出应用（NSIS `/S /UPDATE /R`，成功后安装器自动拉起新版）。
#[tauri::command]
pub fn install_update(app: AppHandle) -> AppResult<()> {
    updater::install(&app)
}

/// 在资源管理器中定位已下载的安装包（安装失败兜底）。
#[tauri::command]
pub fn open_update_installer(app: AppHandle) -> AppResult<()> {
    updater::open_installer_folder(&app)
}

/// 用系统浏览器打开安装包下载地址（下载失败兜底）。
#[tauri::command]
pub fn open_update_download_url(app: AppHandle) -> AppResult<()> {
    updater::open_download_url(&app)
}

fn emit_quick_panel_state(app: &AppHandle, config: &AppConfig) {
    let _ = app.emit_to(
        QUICK_PANEL_LABEL,
        QUICK_PANEL_EVENT,
        QuickPanelState {
            enabled: config.quick_panel,
            edge: config.quick_panel_edge,
            offset: config.quick_panel_y,
        },
    );
}

fn emit_quick_panel_body_state(app: &AppHandle, open: bool, edge: DockEdge) -> AppResult<()> {
    app.emit_to(
        QUICK_PANEL_BODY_LABEL,
        QUICK_PANEL_BODY_EVENT,
        QuickPanelBodyState { open, edge },
    )?;
    Ok(())
}
