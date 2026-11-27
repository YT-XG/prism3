use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_opener::OpenerExt;

use crate::ble::{
    self, AncGradient, BatteryReading, BleCandidate, BleState, BleStateSnapshot, EqPreset,
    NoiseMode,
};
use crate::clipboard::{self, ClipKindFilter, ClipState, ClipboardState};
use crate::config::{AppConfig, AppState, DockEdge};
use crate::db::DbState;
use crate::error::{AppError, AppResult};
use crate::file_index::{self, FileIndexState, IndexState, SearchHit};
use crate::models::{
    Note, NoteFolder, NoteImageGcReport, NoteLinks, NoteTagRow, PurgeReport, Tag, WorkLog,
    WorkProject,
};
use crate::mouse::{self, MouseBattery, MouseState};
use crate::note_image;
use crate::note_io::{self, ExportReport, ImportOptions, ImportReport};
use crate::paste::{self, PasteContent, PasteOutcome};
use crate::phone::{self, PhoneStatus};
use crate::power::{self, ShutdownState, ShutdownStore};
use crate::repo::{note, note_folder, note_link, tag, work_log};
use crate::status::{self, AppStatus, AppStatusState};
use crate::updater::{self, UpdateSnapshot};
use crate::wireless::{self, WirelessBattery, WirelessState};

use std::sync::atomic::Ordering;

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
/// **运行期不要在调用线程上 `build()`**：`WebviewWindowBuilder::build` 会把创建请求
/// 投递给主线程并等它返回，而同步 `#[tauri::command]`（`set_config` / `reset_config`）
/// 恰好跑在主线程上——那就是「主线程等自己」，整个应用会冻住（现场表现：点了没反应、
/// 主窗口也关不掉）。所以缺窗口时把创建丢到一条普通线程；窗口已存在时只广播状态，零开销。
///
/// 例外是 `setup` 里那次直调 `ensure_quick_panel_windows`：那时事件循环还没开跑，
/// 主线程创建窗口是官方支持的路径。
pub fn sync_quick_panel_windows(app: &AppHandle, config: &AppConfig) {
    if !config.quick_panel {
        destroy_quick_panel_windows(app);
        return;
    }

    let missing = app.get_webview_window(QUICK_PANEL_LABEL).is_none()
        || app.get_webview_window(QUICK_PANEL_BODY_LABEL).is_none();
    if !missing {
        emit_quick_panel_state(app, config);
        return;
    }

    let app = app.clone();
    let config = config.clone();
    std::thread::spawn(move || match ensure_quick_panel_windows(&app) {
        Ok(()) => emit_quick_panel_state(&app, &config),
        Err(err) => log::error!("创建快捷小窗窗口失败：{err}"),
    });
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
    // 「定时关机」的两个键只由专用命令写（同 BLE 三键的先例），整份回写不覆盖。
    config.shutdown_time = guard.shutdown_time.clone();
    config.shutdown_weekdays = guard.shutdown_weekdays.clone();
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
    let notes_retention_changed =
        guard.note_trash_retention_days != config.note_trash_retention_days;
    let hub_shortcut_changed = guard.hub_shortcut != config.hub_shortcut;

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
    if notes_retention_changed {
        purge_expired_notes_now(&app);
    }
    if hub_shortcut_changed {
        crate::hotkey::sync(&config.hub_shortcut);
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
    // 默认保留天数为 0（永久保留），这里调用只为保持「配置改了 → 运行时跟着改」一条路径。
    purge_expired_notes_now(&app);
    // 恢复默认是**显式**动作（不是过期副本回写）：BLE 键也要跟着回默认并停掉看门狗。
    ble::sync_from_config(&app);
    // 每天定时关机同样回默认（空 = 关闭）并停掉调度线程。
    power::sync_daily(&app);
    crate::hotkey::sync(&default.hub_shortcut);
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

/// 隐藏组合窗口（`hub`）：前端按 Esc 或点击透明背景时调用。
#[tauri::command]
pub fn hide_hub_window(app: AppHandle) {
    crate::hub::hide(&app);
}

/// 读取全局快捷键的注册状态。
///
/// 注册是系统级的，被别的程序占用或上一次进程还没退干净都会失败（且失败后本会话
/// 一直不生效）。设置页据此把「设了但没生效」和它的原因直接显示出来。
#[tauri::command]
pub fn get_hotkey_state() -> crate::hotkey::HotkeyStatus {
    crate::hotkey::status()
}

/// 把主窗口带到前台：快捷小窗 / 组合窗口点「速记」条目时，主窗口可能正缩在托盘或最小化。
#[tauri::command]
pub fn focus_main_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(err) = window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
        {
            log::warn!("唤起主窗口失败：{err}");
        }
    }
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

/// 打开一个本地项目的目录（资源管理器）。
///
/// 项目路径来自用户配置，走 Rust 侧 opener：前端没有任意路径的 opener 权限。
/// 用系统浏览器打开一个 http(s) 链接（「创建应用」的链接型应用共用项目页那条 opener 链路）。
#[tauri::command]
pub fn open_external_url(app: tauri::AppHandle, url: String) -> AppResult<()> {
    crate::projects::open_url(&app, &url)
}

#[tauri::command]
pub fn open_project_folder(app: AppHandle, path: String) -> AppResult<()> {
    crate::projects::open_folder(&app, &path)
}

/// 在应用内启动一条项目命令（工作目录 = `dir`，空 = 项目目录）。
///
/// 不弹控制台窗口：输出由 `projects.rs` 捕获并经 `project:output` 事件推送，
/// 运行状态经 `project:runs` 广播。用 `async` 属性让 `CreateProcess` 跑在 worker 线程
/// （杀毒软件扫描时可能卡一下）。
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn run_project_command(
    app: AppHandle,
    state: State<'_, crate::projects::ProjectRuns>,
    key: String,
    project_id: String,
    project_name: String,
    command_name: String,
    path: String,
    command: String,
    dir: String,
    env: Option<String>,
) -> AppResult<crate::projects::ProjectRun> {
    crate::projects::start(
        &app,
        &state,
        key,
        project_id,
        project_name,
        command_name,
        &path,
        &command,
        &dir,
        env.as_deref().unwrap_or(""),
    )
}

/// 从项目的 `package.json` 生成命令预设（按锁文件推断 pnpm / npm / yarn / bun）。
#[tauri::command(async)]
pub fn suggest_project_commands(
    path: String,
) -> AppResult<Vec<crate::projects::ProjectCommandSuggestion>> {
    crate::projects::suggest_commands(&path)
}

/// 读取项目的 Git 分支与未提交改动（不是仓库或没装 git 时返回 `isRepo = false`）。
#[tauri::command(async)]
pub fn get_project_git_state(path: String) -> crate::projects::ProjectGitState {
    crate::projects::git_state(&path)
}

/// 启动前的端口预检（本机 IPv4 回环）。
#[tauri::command(async)]
pub fn check_project_port(port: u16) -> crate::projects::PortStatus {
    crate::projects::check_port(port)
}

/// 用系统浏览器打开从命令输出里识别到的本机地址。
#[tauri::command]
pub fn open_project_url(app: AppHandle, url: String) -> AppResult<()> {
    crate::projects::open_url(&app, &url)
}

/// 停止一条运行中的项目命令（结束整棵进程树）。
#[tauri::command(async)]
pub fn stop_project_command(
    app: AppHandle,
    state: State<'_, crate::projects::ProjectRuns>,
    run_id: String,
) -> AppResult<()> {
    crate::projects::stop(&app, &state, &run_id)
}

/// 当前所有项目命令的运行记录（窗口挂载后兜底拉取；变化经 `project:runs` 广播）。
#[tauri::command]
pub fn get_project_runs(
    state: State<'_, crate::projects::ProjectRuns>,
) -> Vec<crate::projects::ProjectRun> {
    crate::projects::snapshot(&state)
}

/// 读取某条运行记录的全部输出（打开控制台时兜底回放；增量经 `project:output` 推送）。
#[tauri::command]
pub fn get_project_run_log(
    state: State<'_, crate::projects::ProjectRuns>,
    run_id: String,
) -> Vec<crate::projects::OutputLine> {
    crate::projects::log_of(&state, &run_id)
}

/// 删除一条已结束的运行记录。
#[tauri::command]
pub fn clear_project_run(
    app: AppHandle,
    state: State<'_, crate::projects::ProjectRuns>,
    run_id: String,
) -> AppResult<()> {
    crate::projects::clear(&app, &state, &run_id)
}

/// 清空某条运行记录的输出缓冲（记录本身保留）。
#[tauri::command]
pub fn clear_project_run_log(state: State<'_, crate::projects::ProjectRuns>, run_id: String) {
    crate::projects::clear_log(&state, &run_id);
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

/// 读取按 CPU / 内存排序的进程列表（监控页每 3 秒轮询一次）。
#[tauri::command(async)]
pub fn get_process_list(
    state: State<'_, crate::process::ProcessState>,
    sort: crate::process::ProcessSort,
    limit: Option<u32>,
) -> Vec<crate::process::ProcessInfo> {
    crate::process::list(&state, sort, limit.unwrap_or(20) as usize)
}

/// 结束一个进程（本应用与系统关键进程受保护）。
#[tauri::command(async)]
pub fn kill_process(state: State<'_, crate::process::ProcessState>, pid: u32) -> AppResult<()> {
    crate::process::kill(&state, pid)
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

/// 读一次鼠标电量（雷神 ML903 接收器，HID feature report；协议见 `mouse.rs`）。
///
/// 由前端卡片按可见性轮询调用：没有设备时也只是几毫秒的枚举，不占常驻线程。
/// 读不到就回「未连接」，不报错（拔掉接收器不该让界面弹错误）。
#[tauri::command(async)]
pub fn read_mouse_battery(state: State<'_, MouseState>) -> AppResult<MouseBattery> {
    Ok(state.store(mouse::read_battery()?))
}

/// 读取缓存的鼠标电量（窗口挂载 / 热重载后兜底拉取）。
#[tauri::command]
pub fn get_mouse_battery(state: State<'_, MouseState>) -> MouseBattery {
    state.current()
}

/// 读一轮多品牌无线外设电量（Logitech HID++ / CompX / 标准 HID；协议见 `wireless.rs`）。
///
/// 由设备页按可见性轮询调用；没有设备时只是一次 HID 枚举，不占常驻线程。
#[tauri::command(async)]
pub fn read_wireless_batteries(state: State<'_, WirelessState>) -> AppResult<Vec<WirelessBattery>> {
    Ok(state.store(wireless::read_all()?))
}

/// 读取缓存的多品牌无线外设电量（窗口挂载 / 热重载后兜底拉取）。
#[tauri::command]
pub fn get_wireless_batteries(state: State<'_, WirelessState>) -> Vec<WirelessBattery> {
    state.current()
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

/// 扫描附近的候选 BLE 设备。
///
/// 返回扫到的**全部**设备（耳机不广播名字、不广播服务 UUID，不带厂商数据就只剩一屏 MAC，
/// 所以每项都带上广播里的 `companyIds`）。`supported` 只表示「命中白名单厂商 ID」，
/// 是提示而不是准入：没命中也允许添加，协议是否相同由连接后的服务 / 特征判定。
/// 适配器不可用是错误，**空列表是正常结果**。
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
pub fn get_clipboard_state(app: AppHandle, state: State<'_, ClipboardState>) -> ClipState {
    state.snapshot(&app)
}

/// 查询剪贴板历史（一页）。空关键词返回「最近入库」的条目。
///
/// `offset` 与 `limit` 一起分页：返回值里带 `total`，前端据此画「第 x / y 页」。
/// 走 worker 线程（`async`）：历史虽小，但图片条目多时线性扫描 + 预览拼接也不该压在主线程上。
#[tauri::command(async)]
pub fn search_clipboard(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    query: String,
    kind: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> clipboard::ClipPage {
    state.query(
        &app,
        &query,
        ClipKindFilter::parse(kind.as_deref()),
        limit,
        offset,
    )
}

/// 取一条文本记录的完整正文（列表只拿预览，编辑 / 展开时才按需拉全文）。
#[tauri::command(async)]
pub fn get_clipboard_text(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: u64,
) -> AppResult<String> {
    state.text_of(&app, id)
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

    if !state.snapshot(&app).paste_on_click {
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

// ------------------------------ 工作记录（完成台账） ------------------------------

/// 工作记录变化的事件名。载荷只带 `revision`，前端据此重拉（与 `notes:changed` 同口径）。
const WORKLOGS_EVENT: &str = "worklogs:changed";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkLogsChanged {
    revision: u64,
}

/// 任何工作记录 / 项目写操作成功后调用：自增 revision 并广播。
fn bump_work_logs(app: &AppHandle) {
    let state = app.state::<DbState>();
    let revision = state.work_revision.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(err) = app.emit(WORKLOGS_EVENT, WorkLogsChanged { revision }) {
        log::warn!("广播工作记录变化失败：{err}");
    }
}

/// 记录类型白名单（与 `db.rs` 的 CHECK 约束一致）。
const WORK_KINDS: [&str; 7] = ["dev", "bug", "support", "doc", "meeting", "ops", "other"];
/// 工作记录状态：done = 已完成，planned = 提前列出的未来工作。
const WORK_STATUSES: [&str; 2] = ["done", "planned"];

fn normalize_work_status(raw: Option<&str>) -> AppResult<String> {
    let status = raw.unwrap_or("done").trim();
    if WORK_STATUSES.contains(&status) {
        Ok(status.to_string())
    } else {
        Err(AppError::Other(format!("未知的工作状态：{status}")))
    }
}

fn normalize_work_kind(raw: Option<&str>) -> AppResult<String> {
    let kind = raw.unwrap_or("dev").trim();
    if WORK_KINDS.contains(&kind) {
        Ok(kind.to_string())
    } else {
        Err(AppError::Other(format!("未知的工作类型：{kind}")))
    }
}

fn validated_work_title(title: &str) -> AppResult<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::Other("请填写完成内容".into()));
    }
    Ok(trimmed.to_string())
}

/// 工时：`0` = 没填；上限 24 小时（一天），保留两位小数。
fn validated_work_hours(hours: Option<f64>) -> AppResult<f64> {
    let value = hours.unwrap_or(0.0);
    if !value.is_finite() || !(0.0..=24.0).contains(&value) {
        return Err(AppError::Other("工时应在 0 到 24 小时之间".into()));
    }
    Ok((value * 100.0).round() / 100.0)
}

/// `[from, to]` 内（按完成时间）的**已完成**工作记录，最新在前。
#[tauri::command(async)]
pub fn list_work_logs(state: State<'_, DbState>, from: u64, to: u64) -> AppResult<Vec<WorkLog>> {
    let conn = state.lock();
    Ok(work_log::list_range(&conn, from, to)?)
}

/// 全部「未来计划」，按计划日期正序。
#[tauri::command(async)]
pub fn list_work_plans(state: State<'_, DbState>) -> AppResult<Vec<WorkLog>> {
    let conn = state.lock();
    Ok(work_log::list_planned(&conn)?)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn create_work_log(
    app: AppHandle,
    state: State<'_, DbState>,
    title: String,
    detail: Option<String>,
    project_id: Option<i64>,
    kind: Option<String>,
    status: Option<String>,
    hours: Option<f64>,
    finished_at: Option<u64>,
) -> AppResult<WorkLog> {
    let title = validated_work_title(&title)?;
    let kind = normalize_work_kind(kind.as_deref())?;
    let status = normalize_work_status(status.as_deref())?;
    let hours = validated_work_hours(hours)?;
    let finished_at = finished_at.unwrap_or_else(crate::db::now_secs);

    let created = {
        let conn = state.lock();
        work_log::create(
            &conn,
            &title,
            detail.as_deref().unwrap_or("").trim(),
            project_id,
            &kind,
            &status,
            hours,
            finished_at,
            "manual",
        )?
    };
    bump_work_logs(&app);
    Ok(created)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn update_work_log(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    title: String,
    detail: Option<String>,
    project_id: Option<i64>,
    kind: Option<String>,
    status: Option<String>,
    hours: Option<f64>,
    finished_at: Option<u64>,
) -> AppResult<WorkLog> {
    let title = validated_work_title(&title)?;
    let kind = normalize_work_kind(kind.as_deref())?;
    let hours = validated_work_hours(hours)?;

    let updated = {
        let conn = state.lock();
        let existing = work_log::get(&conn, id)
            .map_err(|_| AppError::Other("这条工作记录已经不在了，请刷新后重试".to_string()))?;
        let status = match status {
            Some(raw) => normalize_work_status(Some(&raw))?,
            None => existing.status.clone(),
        };
        work_log::update(
            &conn,
            id,
            &title,
            detail.as_deref().unwrap_or("").trim(),
            project_id,
            &kind,
            &status,
            hours,
            finished_at.unwrap_or(existing.finished_at),
        )?
    };
    bump_work_logs(&app);
    Ok(updated)
}

/// 把一条未来计划标记为完成：状态改为 done，完成时间 = 现在。
#[tauri::command]
pub fn complete_work_plan(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
) -> AppResult<WorkLog> {
    let updated = {
        let conn = state.lock();
        work_log::complete(&conn, id, crate::db::now_secs())?
    };
    bump_work_logs(&app);
    Ok(updated)
}

#[tauri::command]
pub fn delete_work_log(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        work_log::delete(&conn, id)?;
    }
    bump_work_logs(&app);
    Ok(())
}

#[tauri::command]
pub fn list_work_projects(state: State<'_, DbState>) -> AppResult<Vec<WorkProject>> {
    let conn = state.lock();
    Ok(work_log::list_projects(&conn)?)
}

#[tauri::command]
pub fn create_work_project(
    app: AppHandle,
    state: State<'_, DbState>,
    name: String,
) -> AppResult<WorkProject> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("请填写项目名称".into()));
    }
    let created = {
        let conn = state.lock();
        if work_log::project_name_taken(&conn, &name)? {
            return Err(AppError::Other(format!("项目「{name}」已存在")));
        }
        work_log::create_project(&conn, &name)?
    };
    bump_work_logs(&app);
    Ok(created)
}

#[tauri::command]
pub fn rename_work_project(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    name: String,
) -> AppResult<()> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("请填写项目名称".into()));
    }
    {
        let conn = state.lock();
        let owner: Option<i64> = conn
            .query_row(
                "SELECT id FROM work_projects WHERE name = ?1 COLLATE NOCASE",
                [&name],
                |r| r.get(0),
            )
            .ok();
        if owner.is_some_and(|owner_id| owner_id != id) {
            return Err(AppError::Other(format!("项目「{name}」已存在")));
        }
        work_log::rename_project(&conn, id, &name)?;
    }
    bump_work_logs(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_work_project(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        work_log::delete_project(&conn, id)?;
    }
    bump_work_logs(&app);
    Ok(())
}

/// 把一段纯文本写进系统剪贴板（周报复制用）。
///
/// 复用 `paste.rs` 的写入链路并登记「这是本应用写的」序列号，避免剪贴板历史
/// 把「复制周报」当成一次新的复制。
#[tauri::command(async)]
pub fn copy_text_to_clipboard(state: State<'_, ClipboardState>, text: String) -> AppResult<()> {
    let seq = paste::copy(&PasteContent::Text(text))?;
    state.note_self_write(seq);
    Ok(())
}

// ------------------------------ 速记（笔记 / 文件夹 / 标签 / 图片） ------------------------------

/// 速记数据变化的事件名。载荷只带 `revision`，前端据此重拉（同 `clipboard:state` 的教训：
/// 不把正文塞进广播里）。
const NOTES_EVENT: &str = "notes:changed";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotesChanged {
    revision: u64,
}

/// 任何笔记侧写操作成功后调用：自增 revision 并广播。
fn bump_notes(app: &AppHandle) {
    let state = app.state::<DbState>();
    let revision = state.notes_revision.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(err) = app.emit(NOTES_EVENT, NotesChanged { revision }) {
        log::warn!("广播速记变化失败：{err}");
    }
}

/// 按配置的保留天数清理速记回收站（`0` = 永久保留）。返回清掉的条数。
fn purge_expired_notes_now(app: &AppHandle) -> usize {
    let days = app
        .state::<AppState>()
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .note_trash_retention_days;
    let state = app.state::<DbState>();
    let conn = state.lock();
    let purged = note::purge_expired(&conn, days as u64).unwrap_or(0);
    if purged > 0 {
        bump_notes(app);
    }
    purged
}

// ---- 笔记 ----

#[tauri::command(async)]
pub fn list_notes(state: State<'_, DbState>, trashed: Option<bool>) -> AppResult<Vec<Note>> {
    let conn = state.lock();
    Ok(note::list_full(&conn, trashed.unwrap_or(false))?)
}

/// 只传 `note_id` 不存在时返回 `None`，其余错误照常上报。
#[tauri::command(async)]
pub fn get_note(state: State<'_, DbState>, note_id: i64) -> AppResult<Option<Note>> {
    let conn = state.lock();
    Ok(note::get(&conn, note_id).ok())
}

#[tauri::command(async)]
pub fn create_note_in(
    app: AppHandle,
    state: State<'_, DbState>,
    title: String,
    content: Option<String>,
    folder_id: Option<i64>,
    source_url: Option<String>,
) -> AppResult<Note> {
    let created = {
        let conn = state.lock();
        note::create_in(
            &conn,
            &title,
            content.as_deref().unwrap_or(""),
            folder_id,
            source_url.as_deref().unwrap_or(""),
        )?
    };
    bump_notes(&app);
    Ok(created)
}

#[tauri::command(async)]
pub fn update_note(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    title: String,
    content: String,
) -> AppResult<Note> {
    let updated = {
        let conn = state.lock();
        note::update_with_link_fixup(&conn, id, &title, &content)?
    };
    bump_notes(&app);
    Ok(updated)
}

#[tauri::command(async)]
pub fn set_note_icon(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    icon: Option<String>,
) -> AppResult<()> {
    {
        let conn = state.lock();
        note::set_icon(&conn, id, icon.as_deref())?;
    }
    bump_notes(&app);
    Ok(())
}

/// 移动单条笔记到文件夹（`folder_id` 传 null = 回树根）。
#[tauri::command(async)]
pub fn set_note_folder(
    app: AppHandle,
    state: State<'_, DbState>,
    note_id: i64,
    folder_id: Option<i64>,
) -> AppResult<()> {
    {
        let conn = state.lock();
        note::set_folder(&conn, note_id, folder_id)?;
    }
    bump_notes(&app);
    Ok(())
}

/// 移入回收站（软删）。UI 删除按钮与撤销恢复走这里；硬删见 `purge_note`。
#[tauri::command(async)]
pub fn trash_note(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        note::trash(&conn, id)?;
    }
    bump_notes(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn restore_note(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        note::restore(&conn, id)?;
    }
    bump_notes(&app);
    Ok(())
}

/// 永久删除（回收站内）：连带 `note_tags`、`note_links` 出链。
#[tauri::command(async)]
pub fn purge_note(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        note::delete(&conn, id)?;
    }
    bump_notes(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn purge_all_trashed_notes(app: AppHandle, state: State<'_, DbState>) -> AppResult<usize> {
    let purged = {
        let conn = state.lock();
        note::purge_all_trashed(&conn)?
    };
    if purged > 0 {
        bump_notes(&app);
    }
    Ok(purged)
}

/// 按设置的保留天数清理回收站（`note_trash_retention_days`，0 = 永久保留）。
#[tauri::command]
pub fn purge_expired_notes(app: AppHandle) -> PurgeReport {
    PurgeReport {
        purged: purge_expired_notes_now(&app),
    }
}

/// 关键词搜索笔记（标题 OR 正文，排除回收站）；全局搜索用。
#[tauri::command(async)]
pub fn search_notes(
    state: State<'_, DbState>,
    keyword: String,
    limit: Option<usize>,
) -> AppResult<Vec<Note>> {
    let conn = state.lock();
    Ok(note::search(
        &conn,
        &keyword,
        limit.unwrap_or(20).clamp(1, 200),
    )?)
}

/// 反链面板数据（出链 + 入链）。
#[tauri::command(async)]
pub fn get_note_links(state: State<'_, DbState>, note_id: i64) -> AppResult<NoteLinks> {
    let conn = state.lock();
    let (outgoing, incoming) = note_link::for_note(&conn, note_id)?;
    Ok(NoteLinks { outgoing, incoming })
}

/// 全库双链索引重建。幂等。
#[tauri::command(async)]
pub fn rebuild_note_links(state: State<'_, DbState>) -> AppResult<()> {
    let conn = state.lock();
    note_link::reindex_all(&conn)?;
    log::info!("速记双链索引已全量重建");
    Ok(())
}

// ---- 文件夹 ----

#[tauri::command(async)]
pub fn list_note_folders(state: State<'_, DbState>) -> AppResult<Vec<NoteFolder>> {
    let conn = state.lock();
    Ok(note_folder::list(&conn)?)
}

#[tauri::command(async)]
pub fn create_note_folder(
    app: AppHandle,
    state: State<'_, DbState>,
    name: String,
    parent_id: Option<i64>,
) -> AppResult<NoteFolder> {
    let folder = {
        let conn = state.lock();
        note_folder::create(&conn, &name, parent_id)?
    };
    bump_notes(&app);
    Ok(folder)
}

#[tauri::command(async)]
pub fn rename_note_folder(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    name: String,
) -> AppResult<()> {
    {
        let conn = state.lock();
        note_folder::rename(&conn, id, &name)?;
    }
    bump_notes(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn delete_note_folder(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        note_folder::delete(&conn, id)?;
    }
    bump_notes(&app);
    Ok(())
}

/// 拖拽落点批量写回（后端做环检测，任一项非法整批拒绝）。
#[tauri::command(async)]
pub fn reorder_note_folders(
    app: AppHandle,
    state: State<'_, DbState>,
    moves: Vec<note_folder::FolderMove>,
) -> AppResult<()> {
    {
        let conn = state.lock();
        note_folder::reorder(&conn, &moves)?;
    }
    bump_notes(&app);
    Ok(())
}

// ---- 标签 ----

#[tauri::command(async)]
pub fn list_tags(state: State<'_, DbState>) -> AppResult<Vec<Tag>> {
    let conn = state.lock();
    Ok(tag::list(&conn)?)
}

#[tauri::command(async)]
pub fn create_tag(app: AppHandle, state: State<'_, DbState>, name: String) -> AppResult<Tag> {
    let created = {
        let conn = state.lock();
        tag::create(&conn, &name)?
    };
    bump_notes(&app);
    Ok(created)
}

#[tauri::command(async)]
pub fn rename_tag(
    app: AppHandle,
    state: State<'_, DbState>,
    id: i64,
    name: String,
) -> AppResult<()> {
    {
        let conn = state.lock();
        tag::rename(&conn, id, &name)?;
    }
    bump_notes(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn delete_tag(app: AppHandle, state: State<'_, DbState>, id: i64) -> AppResult<()> {
    {
        let conn = state.lock();
        tag::delete(&conn, id)?;
    }
    bump_notes(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn get_note_tags(state: State<'_, DbState>, note_id: i64) -> AppResult<Vec<Tag>> {
    let conn = state.lock();
    Ok(tag::tags_of_note(&conn, note_id)?)
}

#[tauri::command(async)]
pub fn set_note_tags(
    app: AppHandle,
    state: State<'_, DbState>,
    note_id: i64,
    tag_ids: Vec<i64>,
) -> AppResult<()> {
    {
        let conn = state.lock();
        tag::set_note_tags(&conn, note_id, &tag_ids)?;
    }
    bump_notes(&app);
    Ok(())
}

/// 笔记-标签全量关联（前端构建 `tag_id → noteIds` 筛选映射用）。
#[tauri::command(async)]
pub fn list_note_tags(state: State<'_, DbState>) -> AppResult<Vec<NoteTagRow>> {
    let conn = state.lock();
    Ok(tag::list_note_tags(&conn)?)
}

// ---- 笔记图片 ----

/// 导入一张笔记图片（前端传 base64），返回正文应插入的 URL。
#[tauri::command(async)]
pub fn import_note_image(app: AppHandle, data_b64: String, ext: String) -> AppResult<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.trim())
        .map_err(|err| AppError::Other(format!("图片解码失败：{err}")))?;
    note_image::save(&app, &bytes, &ext)
}

/// 把一张笔记图片写进系统剪贴板（速记里右键「复制」图片时前端调用）。
///
/// 编辑器默认把图片节点序列化成 Markdown 文本（`![alt](http://prism3-note.localhost/<name>)`），
/// 粘到微信 / 文档里就只是一行字；这里改成写真正的位图，粘到别处就是图片。
/// 写入复用剪贴板历史那条已验证过的链路（`paste::copy`，`arboard` 的 set_image 在
/// 真机上会失败，见 `paste.rs`）。
///
/// `src` 是正文里的图片地址，只接受笔记协议 URL（文件名必须是 16 位哈希 + 白名单扩展，
/// 与协议处理器同一套校验，杜绝路径穿越）与内联 `data:image/...;base64,` 两种。
#[tauri::command(async)]
pub fn copy_note_image(app: AppHandle, src: String) -> AppResult<()> {
    let bytes = read_note_image_bytes(&app, &src)?;
    let rgba = image::load_from_memory(&bytes)
        .map_err(|err| AppError::Other(format!("解码图片失败：{err}")))?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    paste::copy(&paste::PasteContent::Image {
        width,
        height,
        rgba: rgba.into_raw(),
    })?;
    Ok(())
}

/// 按正文里的图片地址取回图片字节。
///
/// 只认两种来源：`src` 是笔记协议 URL 时直接读本地文件（原样字节，不重新编码）；
/// 是内联 `data:image/...;base64,` 时解 base64。外链图片读不到像素（画布会被跨源
/// 污染），直接报错让用户先下载。
fn read_note_image_bytes(app: &AppHandle, src: &str) -> AppResult<Vec<u8>> {
    let src = src.trim();

    if let Some(rest) = src.strip_prefix(note_image::NOTE_URL_PREFIX) {
        let name = rest.split(['?', '#']).next().unwrap_or("");
        if !note_image::valid_note_image_name(name) {
            return Err(AppError::Other("不是可识别的笔记图片".into()));
        }
        let path = note_image::image_dir(app)?.join(name);
        return Ok(std::fs::read(path)?);
    }

    if let Some(rest) = src.strip_prefix("data:image/") {
        use base64::Engine;
        let data = rest
            .split_once("base64,")
            .map(|(_, data)| data.trim())
            .ok_or_else(|| AppError::Other("无法解析图片 data URL".into()))?;
        return base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|err| AppError::Other(format!("图片解码失败：{err}")));
    }

    Err(AppError::Other(
        "只支持复制笔记内的图片；外链图片请先下载到本地".into(),
    ))
}

/// 孤儿图片扫描 / 清理（`dry_run = true` 只报告；`false` 真删）。
/// 回收站内笔记的引用也算活引用，避免「先清孤儿、再还原笔记」丢图。
#[tauri::command(async)]
pub fn gc_orphan_note_images(
    app: AppHandle,
    state: State<'_, DbState>,
    dry_run: bool,
) -> AppResult<NoteImageGcReport> {
    use std::collections::HashSet;

    let dir = note_image::image_dir(&app)?;
    let mut files: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if note_image::valid_note_image_name(name) {
                    files.push(name.to_string());
                }
            }
        }
    }

    let mut referenced: HashSet<String> = HashSet::new();
    {
        let conn = state.lock();
        for content in note::all_contents(&conn)? {
            for stem in note_image::extract_image_stems(&content) {
                referenced.insert(stem);
            }
        }
    }

    let orphans: Vec<String> = files
        .iter()
        .filter(|file| {
            let stem = file.split('.').next().unwrap_or("");
            !referenced.contains(stem)
        })
        .cloned()
        .collect();

    let mut report = NoteImageGcReport {
        dry_run,
        total_files: files.len(),
        referenced: referenced.len(),
        orphan_files: orphans,
        removed: 0,
        failed: 0,
    };
    if !dry_run {
        let mut removed = 0usize;
        let mut failed = 0usize;
        for name in &report.orphan_files {
            if std::fs::remove_file(dir.join(name)).is_ok() {
                removed += 1;
            } else {
                failed += 1;
            }
        }
        report.removed = removed;
        report.failed = failed;
        log::info!(
            "笔记孤儿图片清理: 孤儿 {} 张，删除 {} 张，失败 {} 张",
            report.orphan_files.len(),
            removed,
            failed
        );
    }
    Ok(report)
}

// ---- 速记导出 / 导入 ----

/// 导出全部速记到指定目录（按文件夹树建目录 + 每条一个 .md + front-matter + assets/）。
#[tauri::command(async)]
pub fn export_notes(
    app: AppHandle,
    state: State<'_, DbState>,
    dir: String,
) -> AppResult<ExportReport> {
    // 先读快照再释放锁，文件 IO 全程不持 DB 锁（导出是分钟级操作）。
    let snapshot = {
        let conn = state.lock();
        note_io::read_export_snapshot(&conn)?
    };
    note_io::export(&app, snapshot, std::path::Path::new(&dir))
}

/// 导入本应用导出的目录（front-matter 格式已知）；`sourceUrl` 命中默认跳过，可开覆盖。
#[tauri::command(async)]
pub fn import_notes(
    app: AppHandle,
    state: State<'_, DbState>,
    dir: String,
    opts: Option<ImportOptions>,
) -> AppResult<ImportReport> {
    let report = note_io::import(
        &app,
        &state,
        std::path::Path::new(&dir),
        opts.unwrap_or_default(),
    )?;
    if report.imported > 0 {
        bump_notes(&app);
    }
    Ok(report)
}

/// 取消正在进行的导入（幂等）。
#[tauri::command]
pub fn import_notes_cancel() -> AppResult<()> {
    note_io::cancel_import();
    Ok(())
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

// ---------------------------------------------------------------------------
// 手机（Android）远程控制：adb 截屏 + 触摸 / 按键注入（见 `phone.rs`）
// ---------------------------------------------------------------------------

/// 探测 adb 与已连接设备（手机页挂载时调一次，之后靠「刷新」）。
///
/// 可能现场拉起 adb server（首次 1–2 秒），因此走 `async` 不占 UI 线程。
#[tauri::command(async)]
pub fn phone_status(app: AppHandle) -> PhoneStatus {
    phone::status(&app)
}

/// 一键下载并解压 platform-tools（约 8 MB），装好后立刻重新探测。
///
/// 下载与解压都是阻塞的（reqwest blocking + 系统 `tar.exe`），放到阻塞线程池执行；
/// 失败原样冒到前端（页面据此显示「下载失败」而不是永远转圈）。
#[tauri::command]
pub async fn phone_install_adb(app: AppHandle) -> AppResult<PhoneStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        phone::install_platform_tools(&app)?;
        Ok(phone::status(&app))
    })
    .await
    .map_err(|err| AppError::Other(format!("下载 platform-tools 的任务异常：{err}")))?
}

/// 记住选中的设备（写 `phone_serial`，下次打开手机页直接选中它）。
#[tauri::command]
pub fn phone_select_device(app: AppHandle, serial: String) -> AppResult<()> {
    phone::select_device(&app, &serial)
}

/// 截一张 PNG 原图。返回 `tauri::ipc::Response`：前端拿到的是 `ArrayBuffer`，
/// 不经过 base64（一帧 1–3 MB，编码一次要多花 1/3 的带宽与一次大字符串分配）。
#[tauri::command(async)]
pub fn phone_screencap(app: AppHandle, serial: String) -> AppResult<tauri::ipc::Response> {
    let bytes = phone::screencap(&app, &serial)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 在设备坐标上点一下。
#[tauri::command(async)]
pub fn phone_tap(app: AppHandle, serial: String, x: i32, y: i32) -> AppResult<()> {
    phone::tap(&app, &serial, x, y)
}

/// 从 `(from_x, from_y)` 滑到 `(to_x, to_y)`，`duration_ms` 是滑动时长。
#[tauri::command(async)]
pub fn phone_swipe(
    app: AppHandle,
    serial: String,
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    duration_ms: u32,
) -> AppResult<()> {
    phone::swipe(&app, &serial, (from_x, from_y), (to_x, to_y), duration_ms)
}

/// 发送一个白名单里的按键（`back` / `home` / `recents` / `power` / `wake` / `volumeUp` …）。
#[tauri::command(async)]
pub fn phone_key(app: AppHandle, serial: String, key: phone::PhoneKey) -> AppResult<()> {
    phone::key(&app, &serial, key)
}

/// 老板模式挂件：在画面中央竖滑（`up` = 从下往上，`down` = 从上往下）。
#[tauri::command(async)]
pub fn phone_swipe_vertical(
    app: AppHandle,
    serial: String,
    direction: phone::SwipeDirection,
) -> AppResult<()> {
    phone::swipe_vertical(&app, &serial, direction)
}

/// 锁屏（先 `KEYCODE_SLEEP`，不生效才退到电源键），返回一句可展示的结果说明。
#[tauri::command(async)]
pub fn phone_lock(app: AppHandle, serial: String) -> AppResult<String> {
    phone::lock(&app, &serial)
}

// ---------------------------------------------------------------------------
// 手机无线调试（adb over Wi-Fi）：拔掉数据线，手机就不再一直充电
// ---------------------------------------------------------------------------

/// 无线调试现状。**只读**（`adb devices` + 两条 getprop/ip），可以放心轮询。
///
/// 永远返回结构体、不报错：没有 adb / 没连 Wi-Fi / 没设备都只是字段为 None，
/// 界面上比「弹一个错误」更好用。
#[tauri::command]
pub async fn phone_wireless_info(app: AppHandle) -> phone::WirelessInfo {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_info(&app))
        .await
        .unwrap_or_default()
}

/// 一键切无线：把 adbd 切到 TCP（固定 5555）并立刻连上。
///
/// 需要 USB 那条还在（要它读手机的 Wi-Fi 地址、并下 `adb tcpip` 指令）。
#[tauri::command]
pub async fn phone_wireless_enable(app: AppHandle) -> AppResult<phone::WirelessInfo> {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_enable(&app))
        .await
        .map_err(|err| AppError::Other(format!("无线调试任务异常：{err}")))?
}

/// 自动发现并连上手机（记住的地址 → mDNS 广播 → 局域网邻居表），不用手填 IP。
#[tauri::command]
pub async fn phone_wireless_discover(app: AppHandle) -> AppResult<phone::WirelessInfo> {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_discover(&app))
        .await
        .map_err(|err| AppError::Other(format!("无线调试任务异常：{err}")))?
}

/// 手动连接一个已知地址（`ip:端口`）。
#[tauri::command]
pub async fn phone_wireless_connect(
    app: AppHandle,
    address: String,
) -> AppResult<phone::WirelessInfo> {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_connect(&app, &address))
        .await
        .map_err(|err| AppError::Other(format!("无线调试任务异常：{err}")))?
}

/// 断开无线连接（USB 那条不受影响）。
#[tauri::command]
pub async fn phone_wireless_disconnect(
    app: AppHandle,
    address: String,
) -> AppResult<phone::WirelessInfo> {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_disconnect(&app, &address))
        .await
        .map_err(|err| AppError::Other(format!("无线调试任务异常：{err}")))?
}

/// Android 11+「无线调试」配对（`adb tcpip` 走不通的机型才需要）。
#[tauri::command]
pub async fn phone_wireless_pair(
    app: AppHandle,
    address: String,
    code: String,
) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || phone::wireless_pair(&app, &address, &code))
        .await
        .map_err(|err| AppError::Other(format!("无线调试任务异常：{err}")))?
}

// ---------------------------------------------------------------------------
// 手机「老板模式」：一枚置顶悬浮窗，上面是「同步」+ 上/下滑三枚按键（见 phone-boss.tsx）
// ---------------------------------------------------------------------------

/// 老板模式悬浮窗的 label（前端 `quick-panel.tsx` 按它分派到 `PhoneBossPanel`）。
pub const PHONE_BOSS_LABEL: &str = "phone-boss";

/// 悬浮窗口尺寸（逻辑像素）：比胶囊本身（200×56）大一圈，专门留出投影的余量——
/// 窗口是透明的，投影一旦贴着窗口边缘就会被裁掉，挂件看起来就会「贴」在桌面上。
const PHONE_BOSS_WIDTH: f64 = 232.0;
const PHONE_BOSS_HEIGHT: f64 = 88.0;
/// 胶囊在窗口里的内缩（与 `phone-boss.tsx` 的居中布局一致）。
const PHONE_BOSS_INSET: f64 = 16.0;
/// 胶囊距屏幕右下角的目标留白：右边 28、下边 96（避开任务栏）。
const PHONE_BOSS_MARGIN_RIGHT: f64 = 28.0;
const PHONE_BOSS_MARGIN_BOTTOM: f64 = 96.0;

/// 默认落位：光标所在那块屏的右下角（多屏时出现在鼠标那一块，和组合窗口同一取法）。
fn phone_boss_position(app: &AppHandle) -> (f64, f64) {
    let Some(monitor) = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
    else {
        // 拿不到显示器信息时按 1080p 右下角兜底（几乎不会发生）。
        return (
            1920.0 - PHONE_BOSS_WIDTH - PHONE_BOSS_MARGIN_RIGHT,
            1080.0 - PHONE_BOSS_HEIGHT - PHONE_BOSS_MARGIN_BOTTOM,
        );
    };

    let scale = monitor.scale_factor();
    let origin_x = monitor.position().x as f64 / scale;
    let origin_y = monitor.position().y as f64 / scale;
    let width = monitor.size().width as f64 / scale;
    let height = monitor.size().height as f64 / scale;
    // 留白是给**胶囊**算的，窗口比胶囊大一圈（四周各 `PHONE_BOSS_INSET`），这里减掉。
    (
        origin_x + width - PHONE_BOSS_WIDTH - (PHONE_BOSS_MARGIN_RIGHT - PHONE_BOSS_INSET),
        origin_y + height - PHONE_BOSS_HEIGHT - (PHONE_BOSS_MARGIN_BOTTOM - PHONE_BOSS_INSET),
    )
}

/// 按需创建老板模式悬浮窗：透明、置顶、无边框、不占任务栏、**不抢焦点**
/// （它是一块常年浮在别的窗口上面的小挂件，点按钮时也不该把用户正在用的窗口顶下去）。
///
/// **直接以 `visible(true)` 创建**：早先的做法是 `visible(false)` + 事后 `show()`，
/// 实测出现过「窗口建出来了、`show()` 却没生效」的隐形孤儿窗（之后每次点开关都拿它
/// 再 `show()` 一次，永远打不开）。创建即可见没有这个中间态；万一失败也能直接拿到错误。
fn build_phone_boss_window(app: &AppHandle) -> AppResult<()> {
    let (x, y) = phone_boss_position(app);
    let window = WebviewWindowBuilder::new(
        app,
        PHONE_BOSS_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    // 标题保持中性：这个窗口不在任务栏 / Alt+Tab 里，但 Win+Tab 之类仍可能露出标题。
    .title("Prism3 挂件")
    .position(x, y)
    .inner_size(PHONE_BOSS_WIDTH, PHONE_BOSS_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(true)
    .build()
    .map_err(|err| {
        // 这里必须留痕：`build()` 有可能**先建出原生窗口再失败**（WebView2 没挂上），
        // 窗口会留在屏幕上一片空白，前端只看到一句 toast——只看界面根本判断不出是
        // WebView 没起来还是渲染出错。
        log::warn!("创建老板模式悬浮窗失败（窗口可能已被创建但不可用）：{err}");
        AppError::Other(format!("创建老板模式悬浮窗失败：{err}"))
    })?;
    log::debug!(
        "老板模式悬浮窗已创建：{}x{} @ ({x}, {y}) visible={:?}",
        PHONE_BOSS_WIDTH,
        PHONE_BOSS_HEIGHT,
        window.is_visible()
    );
    Ok(())
}

/// 老板模式悬浮窗是否已打开（手机页挂载时兜底拉取，与开关按钮保持一致）。
#[tauri::command]
pub fn phone_boss_state(app: AppHandle) -> bool {
    app.get_webview_window(PHONE_BOSS_LABEL)
        .map(|window| window.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

/// 打开老板模式悬浮窗（不存在则现场创建），返回**真实的可见状态**。
///
/// **为什么是 `async` + `spawn_blocking`**：`WebviewWindowBuilder::build` 会把创建请求
/// 投递给主线程并等它完成，而同步 `#[tauri::command]` 本身就跑在主线程上——直接建就是
/// 「主线程等自己」，整个应用冻住。实测现场：点了「老板模式」之后所有按钮都没反应、
/// 主窗口也关不掉、手机页永远停在「正在探测」（那条 IPC 的返回永远送不回来）。
/// 运行期任何建/销毁窗口的路径都必须遵守这条（`sync_quick_panel_windows` 同理）。
///
/// 刻意**不对主窗口做任何事**：这一键只是「多一个随手能按的息屏按钮」。
/// 自愈：窗口存在但不可见（旧版本留下的隐形孤儿窗）就先销毁再建。
#[tauri::command]
pub async fn open_phone_boss_window(app: AppHandle) -> AppResult<bool> {
    tauri::async_runtime::spawn_blocking(move || open_phone_boss_blocking(&app))
        .await
        .map_err(|err| AppError::Other(format!("老板模式悬浮窗任务异常：{err}")))?
}

fn open_phone_boss_blocking(app: &AppHandle) -> AppResult<bool> {
    log::debug!("打开老板模式悬浮窗");
    if let Some(window) = app.get_webview_window(PHONE_BOSS_LABEL) {
        if window.is_visible().unwrap_or(false) {
            return Ok(true);
        }
        log::debug!("老板模式悬浮窗存在但不可见，销毁后重建");
        let _ = window.destroy();
    }

    build_phone_boss_window(app)?;
    let window = app
        .get_webview_window(PHONE_BOSS_LABEL)
        .ok_or_else(|| AppError::Other("老板模式悬浮窗创建后找不到窗口".into()))?;

    // 正常情况下 `visible(true)` 创建就已经可见；这里再兜一次（并验证真可见）。
    if !window.is_visible().unwrap_or(false) {
        window
            .show()
            .map_err(|err| AppError::Other(format!("显示老板模式悬浮窗失败：{err}")))?;
    }
    let visible = window.is_visible().unwrap_or(false);
    log::debug!("老板模式悬浮窗已打开 visible={visible}");
    if !visible {
        return Err(AppError::Other(
            "老板模式悬浮窗已创建但没能显示（可检查显示器 / 远程桌面会话）".into(),
        ));
    }
    Ok(true)
}

/// 关闭老板模式悬浮窗：**销毁**而不只是隐藏（不常用，隐藏着的 WebView 白占内存）。
///
/// 同样必须离开主线程：`destroy()` 也要等主线程处理完（原因见上）。
#[tauri::command]
pub async fn close_phone_boss_window(app: AppHandle) -> AppResult<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(window) = app.get_webview_window(PHONE_BOSS_LABEL) {
            if let Err(err) = window.destroy() {
                log::warn!("销毁老板模式悬浮窗失败：{err}");
                return Err(AppError::Other(format!("关闭老板模式悬浮窗失败：{err}")));
            }
        }
        Ok(false)
    })
    .await
    .map_err(|err| AppError::Other(format!("关闭老板模式悬浮窗任务异常：{err}")))?
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

// ---------------------------------------------------------------------------
// 定时关机（见 power.rs）
// ---------------------------------------------------------------------------

/// 读取定时关机状态（页面挂载 / 热重载后兜底拉取：事件不缓存）。
#[tauri::command]
pub fn get_shutdown_state(state: State<'_, ShutdownStore>) -> ShutdownState {
    state.snapshot()
}

/// 设置倒计时关机：`minutes` 分钟后由 Windows 关闭计算机（1 分钟 ~ 24 小时）。
///
/// 倒计时与「每天定时」互斥：开倒计时会把配置里的每天定时一起清掉。
#[tauri::command]
pub fn schedule_shutdown(
    app: AppHandle,
    state: State<'_, ShutdownStore>,
    minutes: u32,
) -> AppResult<ShutdownState> {
    power::schedule(&app, &state, minutes)
}

/// 保存每天定时关机：`time` = `HH:mm`，`weekdays` = 0–6（空 = 每天）。
#[tauri::command]
pub fn set_daily_shutdown(
    app: AppHandle,
    time: String,
    weekdays: Vec<u32>,
) -> AppResult<ShutdownState> {
    power::set_daily(&app, &time, &weekdays)
}

/// 取消关机计划（倒计时与每天定时都适用），并清掉配置里的每天定时。
#[tauri::command]
pub fn cancel_shutdown(
    app: AppHandle,
    state: State<'_, ShutdownStore>,
) -> AppResult<ShutdownState> {
    power::cancel(&app, &state)
}
