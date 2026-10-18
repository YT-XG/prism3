//! 剪贴板「回到上一个焦点并粘贴」。
//!
//! 两个窗口共用同一条链路：
//! 1. 把内容写进系统剪贴板（`arboard`），并记下写入后的剪贴板序列号
//!    （采集线程据此跳过，不把「粘贴」当成一次新的复制）；
//! 2. 目标窗口：**追踪线程**持续记录「最近一个不属于本应用的前台窗口」——
//!    它就是用户心里的「上一个焦点」；
//! 3. 若此刻前台是本应用（主窗口点了记录，或面板处于临时聚焦模式），
//!    先用 `SetForegroundWindow` 把焦点还给那个窗口（必要时借 `AttachThreadInput`
//!    绕过前台切换限制）并校验；前台本来就是别的应用时不必抢；
//! 4. `SendInput` 发一次 Ctrl+V。
//!
//! 快捷小窗的面板窗口平时 `focusable: false`，**压根不会抢走焦点**：悬停着点一条记录时
//! 目标应用一直是前台，第 3 步自然跳过——这也是「点一下就能粘回去」在面板里最顺的原因。
//!
//! `HWND` 不实现 `Send`，静态量里存 `isize`（与 `focus.rs` 同一处理）。

use std::borrow::Cow;

use serde::Serialize;

use crate::error::AppResult;

/// 要写进剪贴板的内容。
#[derive(Debug, Clone)]
pub enum PasteContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// 一次粘贴尝试的结果：即使没粘成功，内容也已经进了剪贴板。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteOutcome {
    pub pasted: bool,
    /// 没能自动粘贴时的原因（前端提示用）。
    pub reason: Option<String>,
}

impl PasteOutcome {
    fn ok() -> Self {
        Self {
            pasted: true,
            reason: None,
        }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self {
            pasted: false,
            reason: Some(reason.into()),
        }
    }

    /// 只复制、不粘贴（配置里关掉了「点击即粘贴」）：不是失败，也就没有原因。
    pub fn copied() -> Self {
        Self {
            pasted: false,
            reason: None,
        }
    }
}

/// 把内容写进系统剪贴板（跨平台部分，Windows 与非 Windows 共用）。
fn write_clipboard(content: &PasteContent) -> AppResult<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    match content {
        PasteContent::Text(text) => clipboard.set_text(text.clone())?,
        PasteContent::Image {
            width,
            height,
            rgba,
        } => clipboard.set_image(arboard::ImageData {
            width: *width as usize,
            height: *height as usize,
            bytes: Cow::Borrowed(rgba),
        })?,
    }
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_CONTROL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
    };

    use super::{write_clipboard, PasteContent, PasteOutcome};

    /// 追踪「上一个焦点」的轮询间隔。
    const TRACK_INTERVAL: Duration = Duration::from_millis(250);
    /// 归还焦点后的等待：前台切换不是瞬时的，立即发按键会打到错误的窗口。
    const ACTIVATE_DELAY: Duration = Duration::from_millis(40);
    /// 归还焦点的最大等待（40ms × 8 ≈ 320ms）。
    const ACTIVATE_ATTEMPTS: u32 = 8;
    /// `V` 键的虚拟键码。
    const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

    /// 最近一个**不属于本应用**的前台窗口。
    static LAST_EXTERNAL: Mutex<Option<isize>> = Mutex::new(None);
    /// 追踪线程的运行标志。
    static TRACKER_RUNNING: AtomicBool = AtomicBool::new(false);
    /// 「是否处于启用状态」——线程退出时据此决定要不要把自己再拉起来。
    static TRACKER_WANTED: AtomicBool = AtomicBool::new(false);

    fn lock() -> std::sync::MutexGuard<'static, Option<isize>> {
        LAST_EXTERNAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 丢弃记录的「上一个焦点」（功能关闭时调用）。
    pub fn forget() {
        *lock() = None;
    }

    /// 起追踪线程（幂等）。剪贴板历史启用期间常驻，开销只是一次极轻量的句柄查询。
    pub fn start_tracker(app: &AppHandle) {
        TRACKER_WANTED.store(true, Ordering::SeqCst);
        if TRACKER_RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        spawn_tracker(app.clone());
    }

    fn spawn_tracker(app: AppHandle) {
        std::thread::spawn(move || {
            while TRACKER_WANTED.load(Ordering::SeqCst) {
                if let Some(foreground) = external_foreground(&app) {
                    *lock() = Some(foreground);
                }
                std::thread::sleep(TRACK_INTERVAL);
            }

            TRACKER_RUNNING.store(false, Ordering::SeqCst);
            // 退出前再确认一次：刚置 false 的瞬间可能又被启用。此时本线程已是「老线程」，
            // 得有人接手；`swap` 保证「接手」在两个线程里只有一个做得成，不会留下两个追踪线程。
            if TRACKER_WANTED.load(Ordering::SeqCst)
                && !TRACKER_RUNNING.swap(true, Ordering::SeqCst)
            {
                spawn_tracker(app);
            }
        });
    }

    /// 停追踪线程（幂等）。
    pub fn stop_tracker() {
        TRACKER_WANTED.store(false, Ordering::SeqCst);
        forget();
    }

    /// 写剪贴板并返回写入后的序列号（供采集线程跳过自己这次写入）。
    pub fn copy(content: &PasteContent) -> crate::error::AppResult<Option<u32>> {
        write_clipboard(content)?;
        // SAFETY: 只读查询序列号，无副作用。
        let seq = unsafe { GetClipboardSequenceNumber() };
        Ok((seq != 0).then_some(seq))
    }

    /// 把焦点还给「上一个焦点」并发一次 Ctrl+V。
    pub fn paste(app: &AppHandle) -> PasteOutcome {
        let Some(target) = *lock() else {
            return PasteOutcome::skipped("没有可粘贴的目标窗口（内容已复制到剪贴板）");
        };
        let target = HWND(target as *mut c_void);
        // SAFETY: 只读查询窗口可见性。
        if !unsafe { IsWindowVisible(target).as_bool() } {
            return PasteOutcome::skipped("上一个窗口已关闭（内容已复制到剪贴板）");
        }

        // 前台本来就是别的应用（面板平时不抢焦点）时不必抢，直接发按键。
        if foreground_is_ours(app) && !activate(target) {
            return PasteOutcome::skipped("无法把焦点还给上一个窗口（内容已复制到剪贴板）");
        }

        if !send_paste_keys() {
            return PasteOutcome::skipped("发送粘贴按键失败（内容已复制到剪贴板）");
        }
        PasteOutcome::ok()
    }

    /// 当前前台窗口是否属于本应用。
    fn foreground_is_ours(app: &AppHandle) -> bool {
        // SAFETY: 只读查询当前前台窗口。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return false;
        }
        app.webview_windows()
            .values()
            .filter_map(|window| window.hwnd().ok())
            .any(|hwnd| hwnd.0 == foreground.0)
    }

    /// 最近一个不属于本应用的前台窗口。
    fn external_foreground(app: &AppHandle) -> Option<isize> {
        // SAFETY: 只读查询当前前台窗口。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return None;
        }
        if foreground_is_ours(app) {
            return None;
        }
        Some(foreground.0 as isize)
    }

    /// 把目标窗口切到前台并**校验**结果。
    ///
    /// `SetForegroundWindow` 有准入规则：跨进程切换常常被系统拒绝，因此先
    /// `AttachThreadInput` 把输入队列挂到目标线程上（这是绕过该限制的标准做法），
    /// 切换后再轮询确认前台真的变了，避免把 Ctrl+V 发到错误窗口。
    fn activate(target: HWND) -> bool {
        // SAFETY: 全部是只读查询 + 前台切换，句柄来自本进程记录的系统窗口句柄。
        unsafe {
            let current_thread = GetCurrentThreadId();
            let target_thread = GetWindowThreadProcessId(target, None);
            let attached = target_thread != 0
                && target_thread != current_thread
                && AttachThreadInput(current_thread, target_thread, true).as_bool();

            let _ = SetForegroundWindow(target);

            if attached {
                let _ = AttachThreadInput(current_thread, target_thread, false);
            }

            for _ in 0..ACTIVATE_ATTEMPTS {
                if GetForegroundWindow() == target {
                    return true;
                }
                std::thread::sleep(ACTIVATE_DELAY);
            }
            GetForegroundWindow() == target
        }
    }

    fn key_input(key: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 发一次 Ctrl+V（按下 Ctrl → 按下 V → 松开 V → 松开 Ctrl）。
    fn send_paste_keys() -> bool {
        let inputs = [
            key_input(VK_CONTROL, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL, true),
        ];
        // SAFETY: 使用固定长度的合法输入数组调用 SendInput。
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        sent == inputs.len() as u32
    }
}

#[cfg(not(windows))]
mod imp {
    use tauri::AppHandle;

    use super::{write_clipboard, PasteContent, PasteOutcome};

    /// 非 Windows 不做前台窗口追踪（本项目只发布 Windows，仅为可编译性保留）。
    pub fn forget() {}

    pub fn start_tracker(_app: &AppHandle) {}

    pub fn stop_tracker() {}

    pub fn copy(content: &PasteContent) -> crate::error::AppResult<Option<u32>> {
        write_clipboard(content)?;
        Ok(None)
    }

    pub fn paste(_app: &AppHandle) -> PasteOutcome {
        PasteOutcome::skipped("当前平台不支持自动粘贴（内容已复制到剪贴板）")
    }
}

pub use imp::{copy, paste, start_tracker, stop_tracker};
