//! 组合窗口：一个铺满整屏的透明悬浮窗，结构参照 Windows Game Bar —— 整屏先压一层
//! 变灰蒙版（`hub-panel.tsx` 根节点的半透明黑底），蒙版上放「顶部按钮条（切换主视图）
//! + 左侧一列常驻小视图（系统资源 / 时钟 / 耳机） + 右侧主视图」（快速搜索 / 速记 /
//! 剪贴板 / 项目）。点蒙版空白处、按 Esc、失焦都会收起。
//!
//! 窗口按需创建（不常驻），复用的 HTML 入口是 `quick-panel.html`；前端按窗口
//! label = `hub` 分派到 `HubPanel`。唤起靠全局快捷键（见 `hotkey.rs`），
//! 失焦即隐藏（见 `lib.rs` 的窗口事件），因此它更像一个悬浮面板而不是文档窗口。

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::error::AppResult;

/// 组合窗口的 label（与前端 `quick-panel.tsx` 的分派一致）。
pub const HUB_LABEL: &str = "hub";

/// 唤起 / 收起：已可见且在前台时收起，否则铺满光标所在显示器并聚焦。
pub fn toggle(app: &AppHandle) -> AppResult<()> {
    let window = match app.get_webview_window(HUB_LABEL) {
        Some(window) => window,
        None => {
            // 首次唤起要现场建窗口（含 WebView），dev 下还要等 Vite 编译 quick-panel.html，
            // 慢到秒级是正常的。记一行，方便把「按了没反应」和「注册失败」区分开。
            log::debug!("首次唤起组合窗口：正在创建窗口");
            build(app)?
        }
    };

    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        log::debug!("收起组合窗口");
        window.hide()?;
        return Ok(());
    }

    // 每次显示都重新铺满「光标所在的那块屏」：多屏 / 分辨率 / 缩放变化后旧几何可能已经失效。
    cover_cursor_monitor(app, &window);
    window.show()?;
    // 隐藏状态下的 setSize 可能不生效（与快捷小窗灰条同一个坑），显示后再落实一次。
    cover_cursor_monitor(app, &window);
    log::debug!("显示组合窗口");
    if let Err(err) = window.set_focus() {
        // 抢不到前台不算致命：窗口是 always-on-top，用户仍然看得到。
        log::warn!("组合窗口置前失败（可能被系统前台锁拦下）：{err}");
    }
    Ok(())
}

/// 隐藏组合窗口（前端 Esc / 点击蒙版空白处时调用）。
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(HUB_LABEL) {
        if let Err(err) = window.hide() {
            log::warn!("隐藏组合窗口失败：{err}");
        }
    }
}

fn build(app: &AppHandle) -> AppResult<WebviewWindow> {
    let (x, y, width, height) = monitor_area(app);
    let window = WebviewWindowBuilder::new(
        app,
        HUB_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 组合窗口")
    .position(x, y)
    .inner_size(width, height)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .visible(false)
    // 一显示就进入可输入状态：搜索 / 速记 / 剪贴板都有输入框。
    .focused(true)
    .build()?;
    Ok(window)
}

/// 把窗口铺满光标所在的显示器（取不到就退主屏）；窗口是那层「变灰蒙版」，必须整屏。
fn cover_cursor_monitor(app: &AppHandle, window: &WebviewWindow) {
    let (x, y, width, height) = monitor_area(app);
    if let Err(err) = window.set_position(LogicalPosition::new(x, y)) {
        log::warn!("定位组合窗口失败：{err}");
    }
    if let Err(err) = window.set_size(LogicalSize::new(width, height)) {
        log::warn!("调整组合窗口尺寸失败：{err}");
    }
}

/// 光标所在显示器的逻辑像素区域 `(x, y, width, height)`。
///
/// 取光标所在的屏而不是主屏：多屏时快捷键唤起的悬浮窗应该出现在鼠标那一块。
fn monitor_area(app: &AppHandle) -> (f64, f64, f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        // 兜底：拿不到显示器信息时按 1080p 铺满（几乎不会发生）。
        return (0.0, 0.0, 1920.0, 1080.0);
    };

    let scale = monitor.scale_factor();
    let position = monitor.position();
    let size = monitor.size();
    (
        position.x as f64 / scale,
        position.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    )
}
