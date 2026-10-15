use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, State, WebviewUrl, WebviewWindowBuilder,
};

use crate::config::{AppConfig, AppState, DockEdge};
use crate::error::{AppError, AppResult};
use crate::status::{self, AppStatus, AppStatusState};

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
