use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl,
    WebviewWindowBuilder,
};

use crate::config::{AppConfig, AppState, DockEdge};
use crate::error::{AppError, AppResult};

/// 快捷小窗**灰条**窗口标签。灰条独占交互与状态。
const QUICK_PANEL_LABEL: &str = "quick-panel";
/// 快捷小窗**面板**窗口标签。高度恒定、宽度随页面变化，仅由灰条窗口驱动。
const QUICK_PANEL_BODY_LABEL: &str = "quick-panel-body";
/// 通知灰条窗口开关/边缘/偏移变化的事件名。
const QUICK_PANEL_EVENT: &str = "quick-panel:state";
/// 通知面板窗口播放展开/收起动画的事件名。
const QUICK_PANEL_BODY_EVENT: &str = "quick-panel:body-state";

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

/// 按需创建灰条窗口 `quick-panel`（透明、置顶、不占任务栏、不可聚焦，仅拖动/换边时改几何）。
fn build_bar_window(app: &AppHandle) -> AppResult<()> {
    WebviewWindowBuilder::new(
        app,
        QUICK_PANEL_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 快捷小窗")
    .inner_size(6.0, 64.0)
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

/// 按需创建面板窗口 `quick-panel-body`（初始 360×520，高度恒定；宽度随页面加宽，仅由灰条窗口驱动）。
fn build_body_window(app: &AppHandle) -> AppResult<()> {
    WebviewWindowBuilder::new(
        app,
        QUICK_PANEL_BODY_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 快捷小窗面板")
    .inner_size(360.0, 520.0)
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

/// 读取当前配置。
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 保存配置，同时更新内存状态与磁盘文件。
///
/// `quick_panel_edge` 与 `quick_panel_y` 由灰条窗口独占写入，这里以内存态为准，
/// 避免主窗口持有的旧值把面板拖动后的位置覆盖掉。
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
    let panel_toggled = guard.quick_panel != config.quick_panel;

    *guard = config.clone();
    drop(guard);

    crate::config::save(&app, &config)?;

    if panel_toggled {
        sync_quick_panel_windows(&app, &config);
    }
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
    Ok(default)
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

    // 先位置后尺寸：set_size 会固定左上角向右下扩展，若先改尺寸，
    // 吸附右边缘时窗口会瞬时越过工作区右界（多显示器下溢到右侧副屏，
    // 触发 DPI/缩放切换而抖动）。先定位则中间态只会在工作区内留出
    // 至多一帧的缝隙，不会跨到相邻显示器。
    window.set_position(LogicalPosition::new(x, y))?;
    window.set_size(LogicalSize::new(width, height))?;
    Ok(())
}

/// 仅移动**面板窗口**：拖动灰条时逐帧跟随，尺寸恒定（`PANEL_WIDTH × PANEL_HEIGHT`）故不设尺寸。
#[tauri::command]
pub fn set_quick_panel_body_position(app: AppHandle, x: f64, y: f64) -> AppResult<()> {
    quick_panel_body(&app)?.set_position(LogicalPosition::new(x, y))?;
    Ok(())
}

/// 一次调用重定位并缩放**面板窗口**：宽度随页面变化（帮助页加宽），高度不变。
///
/// 与 `set_quick_panel_body_position` 的区别是这里同时改尺寸；面板窗口为
/// `resizable(false)`，但程序化 `set_size` 仍生效（灰条窗口换边时已依赖同一行为）。
#[tauri::command]
pub fn set_quick_panel_body_geometry(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> AppResult<()> {
    let window = quick_panel_body(&app)?;
    // 先定位后改尺寸：set_size 固定左上角向右下扩展，先定位可保证锚定边（右/上）不越过工作区。
    window.set_position(LogicalPosition::new(x, y))?;
    window.set_size(LogicalSize::new(width, height))?;
    Ok(())
}

/// 展开面板：按当前页宽度定位 + 缩放 → 显示 → 通知面板窗口播放展开动画。
///
/// 面板窗口在收起（隐藏）期间不再同步尺寸，故这里必须一并落实宽度，
/// 否则按帮助页（加宽）重新展开时会沿用上次的旧尺寸。
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
    // 隐藏窗口上的 set_size 可能不生效，show 前后各落实一次，避免旧尺寸残留。
    window.set_position(LogicalPosition::new(x, y))?;
    window.set_size(LogicalSize::new(width, height))?;
    window.show()?;
    window.set_position(LogicalPosition::new(x, y))?;
    window.set_size(LogicalSize::new(width, height))?;
    emit_quick_panel_body_state(&app, true, edge)?;
    Ok(())
}

/// 收起面板：只通知面板窗口播放收起动画，动画结束后由面板窗口自行隐藏。
#[tauri::command]
pub fn close_quick_panel_body(app: AppHandle, edge: DockEdge) -> AppResult<()> {
    emit_quick_panel_body_state(&app, false, edge)
}

/// 隐藏面板窗口（面板窗口在收起动画结束后调用；灰条窗口在禁用兜底时也会调用）。
#[tauri::command]
pub fn hide_quick_panel_body(app: AppHandle) -> AppResult<()> {
    quick_panel_body(&app)?.hide()?;
    Ok(())
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
