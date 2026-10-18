//! 快捷小窗面板（`quick-panel-body`）的**输入焦点模式**。
//!
//! 面板窗口平时以 `focusable: false` 创建（`commands.rs::build_body_window`）：悬停划过
//! 灰条不会抢走当前应用的键盘焦点。代价是**窗口里打不了字**——WebView 的键盘输入
//! 需要宿主窗口真的持有焦点。
//!
//! 于是采用「临时聚焦」：用户点击搜索框时才把窗口切到可聚焦并索要焦点，退出时
//! 把焦点还给进入前的那个系统前台窗口，再把可聚焦性收回去。这样：
//! - 不点输入框 ⇒ 焦点行为与原来完全一致（结构保证，而不是靠时序小心）；
//! - 面板不可见 ⇒ 一定不可聚焦（`release` 由隐藏路径与失焦事件双保险调用）。
//!
//! `HWND` 不实现 `Send`，因此静态量里存 `isize`，用的时候再转回来。

use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewWindow};

/// 面板窗口标签，与 `commands.rs` 保持一致。
const PANEL_LABEL: &str = "quick-panel-body";

/// 进入聚焦模式前的系统前台窗口（退出时归还焦点）。
static RESTORE: Mutex<Option<isize>> = Mutex::new(None);

fn panel(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(PANEL_LABEL)
}

/// 丢弃记录的「原前台窗口」（窗口被销毁 / 功能被关闭时调用）。
pub fn forget() {
    *RESTORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsWindowVisible, SetForegroundWindow,
    };

    use super::{forget, panel, RESTORE};
    use crate::error::{AppError, AppResult};

    /// 进入聚焦模式：面板窗口变为可聚焦并拿到键盘焦点。
    ///
    /// 返回是否真的聚焦成功——系统可能拒绝前台切换（`SetForegroundWindow` 有准入规则），
    /// 失败时把可聚焦性收回去并返回 `false`，由前端提示用户再点一次，而不是静默吞掉。
    pub fn promote(app: &AppHandle) -> AppResult<bool> {
        let window = panel(app).ok_or_else(|| AppError::Other("快捷小窗面板窗口不存在".into()))?;

        if window.is_focused().unwrap_or(false) {
            return Ok(true);
        }

        // 只在「从不可聚焦进入聚焦」时记录原前台窗口，避免重入时把面板自己记进去。
        {
            let mut restore = RESTORE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if restore.is_none() {
                *restore = foreground_of_other_window(app);
            }
        }

        window.set_focusable(true)?;
        window.set_focus()?;

        if !window.is_focused().unwrap_or(false) {
            // 等待一拍再重试：刚改完扩展样式时前台切换可能被系统拒绝。
            std::thread::sleep(Duration::from_millis(60));
            if !window.is_focused().unwrap_or(false) {
                if let Ok(hwnd) = window.hwnd() {
                    // SAFETY: hwnd 属于本进程持有的窗口。
                    unsafe {
                        let _ = SetForegroundWindow(HWND(hwnd.0));
                    }
                }
                let _ = window.set_focus();
            }
        }

        if window.is_focused().unwrap_or(false) {
            return Ok(true);
        }

        // 聚焦失败：不要停在「可被激活但没焦点」的中间态。
        log::warn!("快捷小窗面板进入聚焦模式失败：系统拒绝了前台切换");
        let _ = window.set_focusable(false);
        forget();
        Ok(false)
    }

    /// 退出聚焦模式：先把焦点还给用户原来的应用，再收回可聚焦性。幂等。
    pub fn release(app: &AppHandle) -> AppResult<()> {
        let previous = RESTORE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        let Some(window) = panel(app) else {
            return Ok(());
        };

        if let (Some(previous), Ok(hwnd)) = (previous, window.hwnd()) {
            let previous = HWND(previous as *mut c_void);
            // SAFETY: previous 是本进程此前记录的前台窗口句柄；只读查询与前台切换均安全。
            let foreground = unsafe { GetForegroundWindow() };
            let still_visible = unsafe { IsWindowVisible(previous) }.as_bool();
            // 只有面板此刻确实是前台窗口才归还焦点：否则会平白激活另一个窗口。
            if foreground.0 == hwnd.0 && still_visible {
                unsafe {
                    let _ = SetForegroundWindow(previous);
                }
            }
        }

        window.set_focusable(false)?;
        Ok(())
    }

    /// 当前系统前台窗口，若它属于本应用则返回 `None`。
    fn foreground_of_other_window(app: &AppHandle) -> Option<isize> {
        // SAFETY: 只读查询当前前台窗口。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return None;
        }
        for window in app.webview_windows().values() {
            if let Ok(hwnd) = window.hwnd() {
                if hwnd.0 == foreground.0 {
                    return None;
                }
            }
        }
        Some(foreground.0 as isize)
    }
}

#[cfg(not(windows))]
mod imp {
    use tauri::AppHandle;

    use super::panel;
    use crate::error::{AppError, AppResult};

    /// 非 Windows 退化为「切可聚焦 + 索要焦点」（本项目只发布 Windows，仅为可编译性保留）。
    pub fn promote(app: &AppHandle) -> AppResult<bool> {
        let window = panel(app).ok_or_else(|| AppError::Other("快捷小窗面板窗口不存在".into()))?;
        window.set_focusable(true)?;
        window.set_focus()?;
        Ok(window.is_focused().unwrap_or(false))
    }

    /// 非 Windows 退化为直接收回可聚焦性。
    pub fn release(app: &AppHandle) -> AppResult<()> {
        super::forget();
        if let Some(window) = panel(app) {
            window.set_focusable(false)?;
        }
        Ok(())
    }
}

pub use imp::{promote, release};
