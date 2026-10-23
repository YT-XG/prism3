//! 组合窗口的全局快捷键。
//!
//! 不引入 `tauri-plugin-global-shortcut`：`windows` crate 已是既有依赖，
//! 这里直接用 `RegisterHotKey` 起一条常驻消息循环线程。快捷键变更只向该线程
//! 投递一条消息重新注册，线程本身不重启（避免反复起停线程与注册竞态）。
//!
//! 快捷键字符串形如 `Ctrl+Shift+C`（`Ctrl` / `Alt` / `Shift` / `Win` 四种修饰键 +
//! 字母 / 数字 / F1-F24 / 少量命名键），空串表示禁用。解析与注册失败只记日志，
//! 不影响其余配置保存。

#[cfg(not(windows))]
pub fn init(_app: &tauri::AppHandle, _accelerator: &str) {}

#[cfg(not(windows))]
pub fn sync(_accelerator: &str) {}

#[cfg(windows)]
pub use imp::{init, sync};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use tauri::AppHandle;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT, MOD_WIN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY,
    };

    /// 每个线程内唯一的快捷键 id（注册 / 注销成对出现）。
    const HOTKEY_ID: i32 = 1;
    /// 配置变更后让快捷键线程重新注册的消息。
    const WM_RELOAD: u32 = WM_APP + 1;

    /// 快捷键线程 id；`0` = 尚未启动。
    static THREAD_ID: AtomicU32 = AtomicU32::new(0);
    /// 期望生效的快捷键（由配置驱动，线程按需读取）。
    static WANTED: Mutex<String> = Mutex::new(String::new());

    /// 启动快捷键线程并注册当前快捷键（幂等：已启动则只更新）。
    pub fn init(app: &AppHandle, accelerator: &str) {
        set_wanted(accelerator);
        if THREAD_ID.load(Ordering::SeqCst) == 0 {
            spawn_worker(app.clone());
        } else {
            notify_reload();
        }
    }

    /// 更新期望的快捷键并让线程重新注册（配置变更时调用）。
    pub fn sync(accelerator: &str) {
        set_wanted(accelerator);
        notify_reload();
    }

    fn set_wanted(accelerator: &str) {
        let mut wanted = WANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *wanted = accelerator.trim().to_string();
    }

    fn spawn_worker(app: AppHandle) {
        let result = std::thread::Builder::new()
            .name("prism3-hotkey".into())
            .spawn(move || unsafe {
                // 先建消息队列：拿到线程 id 之后 `notify_reload` 投递的消息才不会丢。
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_NOREMOVE);
                THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);

                apply_wanted();

                loop {
                    let result = GetMessageW(&mut msg, None, 0, 0);
                    if result.0 <= 0 {
                        break;
                    }
                    match msg.message {
                        WM_HOTKEY => {
                            let main_app = app.clone();
                            // 窗口创建 / 显隐必须在主线程执行。
                            let _ = app.run_on_main_thread(move || {
                                if let Err(err) = crate::hub::toggle(&main_app) {
                                    log::warn!("切换组合窗口失败：{err}");
                                }
                            });
                        }
                        WM_RELOAD => apply_wanted(),
                        _ => {}
                    }
                }

                let _ = UnregisterHotKey(None, HOTKEY_ID);
                THREAD_ID.store(0, Ordering::SeqCst);
            });

        if let Err(err) = result {
            log::warn!("启动全局快捷键线程失败：{err}");
        }
    }

    fn notify_reload() {
        let thread_id = THREAD_ID.load(Ordering::SeqCst);
        if thread_id == 0 {
            return;
        }
        if let Err(err) = unsafe {
            PostThreadMessageW(thread_id, WM_RELOAD, WPARAM(0), LPARAM(0))
        } {
            log::warn!("通知快捷键线程失败：{err}");
        }
    }

    /// 按 `WANTED` 重新注册：先注销旧组合，再注册新组合。
    unsafe fn apply_wanted() {
        let _ = UnregisterHotKey(None, HOTKEY_ID);
        let wanted = WANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if wanted.is_empty() {
            return;
        }
        match parse(&wanted) {
            Some((modifiers, key)) => {
                if let Err(err) = RegisterHotKey(None, HOTKEY_ID, modifiers, key) {
                    log::warn!("注册全局快捷键 {wanted} 失败（可能被其它程序占用）：{err}");
                }
            }
            None => log::warn!("无法解析全局快捷键：{wanted}"),
        }
    }

    /// 解析 `Ctrl+Shift+C` 之类的字符串；必须含至少一个修饰键与一个主键。
    fn parse(input: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
        let mut modifiers = 0u32;
        let mut key = None;

        for token in input
            .split('+')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
        {
            match token.as_str() {
                "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
                "alt" => modifiers |= MOD_ALT.0,
                "shift" => modifiers |= MOD_SHIFT.0,
                "win" | "meta" | "super" | "cmd" => modifiers |= MOD_WIN.0,
                other => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(key_code(other)?);
                }
            }
        }

        if modifiers == 0 {
            return None;
        }
        // `MOD_NOREPEAT`：长按不重复触发，否则窗口会开关抖动。
        Some((HOT_KEY_MODIFIERS(modifiers | MOD_NOREPEAT.0), key?))
    }

    fn key_code(token: &str) -> Option<u32> {
        let mut chars = token.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            let upper = ch.to_ascii_uppercase();
            return upper.is_ascii_alphanumeric().then_some(upper as u32);
        }

        if let Some(rest) = token.strip_prefix('f') {
            if let Ok(number) = rest.parse::<u32>() {
                if (1..=24).contains(&number) {
                    return Some(0x70 + number - 1);
                }
            }
        }

        let code = match token {
            "space" => 0x20,
            "enter" | "return" => 0x0D,
            "tab" => 0x09,
            "esc" | "escape" => 0x1B,
            "backspace" => 0x08,
            "delete" | "del" => 0x2E,
            "insert" | "ins" => 0x2D,
            "home" => 0x24,
            "end" => 0x23,
            "pageup" | "pgup" => 0x21,
            "pagedown" | "pgdn" => 0x22,
            "up" => 0x26,
            "down" => 0x28,
            "left" => 0x25,
            "right" => 0x27,
            _ => return None,
        };
        Some(code)
    }
}
