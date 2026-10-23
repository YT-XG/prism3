//! 组合窗口：一个透明大窗口里并排放四张卡片（快速搜索 / 速记 / 剪贴板 / 项目）。
//!
//! 窗口按需创建（不常驻），复用的 HTML 入口是 `quick-panel.html`；前端按窗口
//! label = `hub` 分派到 `HubPanel`。唤起靠全局快捷键（见 `hotkey.rs`），
//! 失焦即隐藏（见 `lib.rs` 的窗口事件），因此它更像一个悬浮面板而不是文档窗口。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::error::AppResult;

/// 组合窗口的 label（与前端 `quick-panel.tsx` 的分派一致）。
pub const HUB_LABEL: &str = "hub";

/// 唤起 / 收起：已可见且在前台时收起，否则居中显示并聚焦。
pub fn toggle(app: &AppHandle) -> AppResult<()> {
    let window = match app.get_webview_window(HUB_LABEL) {
        Some(window) => window,
        None => build(app)?,
    };

    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        window.hide()?;
        return Ok(());
    }

    // 每次显示都重新居中：多屏 / 分辨率变化后旧位置可能已经落在屏幕外。
    window.center()?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

/// 隐藏组合窗口（前端 Esc / 点击透明背景时调用）。
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(HUB_LABEL) {
        if let Err(err) = window.hide() {
            log::warn!("隐藏组合窗口失败：{err}");
        }
    }
}

fn build(app: &AppHandle) -> AppResult<WebviewWindow> {
    let (width, height) = window_size(app);
    let window = WebviewWindowBuilder::new(
        app,
        HUB_LABEL,
        WebviewUrl::App("quick-panel.html".into()),
    )
    .title("Prism3 组合窗口")
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

/// 按主屏工作区估算窗口尺寸：留出边距，四张卡片才排得开。
fn window_size(app: &AppHandle) -> (f64, f64) {
    let logical = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            (
                monitor.size().width as f64 / scale,
                monitor.size().height as f64 / scale,
            )
        })
        .unwrap_or((1920.0, 1080.0));

    (
        (logical.0 * 0.94).clamp(960.0, 1680.0),
        (logical.1 * 0.88).clamp(600.0, 920.0),
    )
}
