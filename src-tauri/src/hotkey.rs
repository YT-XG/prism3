//! 组合窗口的全局快捷键。
//!
//! 不引入 `tauri-plugin-global-shortcut`：`windows` crate 已是既有依赖，
//! 这里直接用 `RegisterHotKey` 起一条常驻消息循环线程。快捷键变更只向该线程
//! 投递一条消息重新注册，线程本身不重启（避免反复起停线程与注册竞态）。
//!
//! 快捷键字符串形如 `Ctrl+Shift+C`（`Ctrl` / `Alt` / `Shift` / `Win` 四种修饰键 +
//! 字母 / 数字 / F1-F24 / 少量命名键），空串表示禁用。解析与注册失败只记日志，
//! 不影响其余配置保存。
//!
//! **注册是系统级的**：`RegisterHotKey` 失败最常见的原因是「热键已注册」
//! （`0x80070581`）——上一次进程还没退干净（`tauri dev` 改 Rust 代码会重启进程，
//! 新旧进程可能重叠一小会儿），或者别的程序占着这个组合键。失败后本会话就一直
//! 不生效，因此这里对**注册失败**做有界重试（见 `imp::schedule_retry`），并把
//! 结果通过 `hotkey:state` 事件 + `get_hotkey_state` 命令暴露出去——否则用户只会
//! 看到「按快捷键没反应」，既没提示也没线索。

/// 快捷键的当前状态（`get_hotkey_state` 的返回值，也是 `hotkey:state` 事件的载荷）。
///
/// 设置页据此告诉用户「设了到底有没有生效」：注册失败在这里有明确原因，
/// 而不是像以前那样只在日志里留一行。
#[derive(Clone, serde::Serialize)]
pub struct HotkeyStatus {
    /// 期望生效的组合键；空串 = 用户主动清除，不再注册。
    pub shortcut: String,
    /// 是否已经在系统里注册成功并生效。
    pub registered: bool,
    /// 未生效的原因（注册失败 / 无法解析）。`None` = 没出问题。
    pub error: Option<String>,
}

#[cfg(not(windows))]
pub fn init(_app: &tauri::AppHandle, _accelerator: &str) {}

#[cfg(not(windows))]
pub fn sync(_accelerator: &str) {}

/// 非 Windows 平台不注册全局快捷键（本项目的目标平台是 Windows）。
#[cfg(not(windows))]
pub fn status() -> HotkeyStatus {
    HotkeyStatus {
        shortcut: String::new(),
        registered: false,
        error: None,
    }
}

#[cfg(windows)]
pub use imp::{init, status, sync};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::{AppHandle, Emitter};
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT, MOD_WIN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY,
    };

    use super::HotkeyStatus;

    /// 每个线程内唯一的快捷键 id（注册 / 注销成对出现）。
    const HOTKEY_ID: i32 = 1;
    /// 配置变更后让快捷键线程重新注册的消息。
    const WM_RELOAD: u32 = WM_APP + 1;
    /// 注册失败后延迟重投一次注册的消息。
    const WM_RETRY: u32 = WM_APP + 2;
    /// 注册失败后的重试间隔。
    const RETRY_INTERVAL: Duration = Duration::from_millis(1000);
    /// 一次注册失败后最多重试几次：4 次 ≈ 覆盖启动期几秒的竞态，又不至于长期和
    /// 别的程序抢同一个组合键（那种情况应该让用户换个键，见 `schedule_retry`）。
    const RETRY_ATTEMPTS: u32 = 4;
    /// 注册状态变化事件名，与前端 `settings-panel.tsx` 一致。
    const STATUS_EVENT: &str = "hotkey:state";

    /// 快捷键线程 id；`0` = 尚未启动。
    static THREAD_ID: AtomicU32 = AtomicU32::new(0);
    /// 期望生效的快捷键与它的真实状态（线程按需读取，设置页也会读）。
    static STATUS: Mutex<HotkeyStatus> = Mutex::new(HotkeyStatus {
        shortcut: String::new(),
        registered: false,
        error: None,
    });
    /// 还剩几次重试额度。
    static RETRY_LEFT: AtomicU32 = AtomicU32::new(RETRY_ATTEMPTS);

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

    /// 当前注册状态（设置页打开时拉一次，之后靠 `hotkey:state` 事件跟随）。
    pub fn status() -> HotkeyStatus {
        lock().clone()
    }

    fn lock() -> std::sync::MutexGuard<'static, HotkeyStatus> {
        STATUS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 记录用户期望的组合键。真实的注册结果由快捷键线程回写（`publish`）。
    fn set_wanted(accelerator: &str) {
        let shortcut = accelerator.trim().to_string();
        let mut status = lock();
        status.shortcut = shortcut;
        status.registered = false;
        status.error = None;
    }

    fn spawn_worker(app: AppHandle) {
        let result = std::thread::Builder::new()
            .name("prism3-hotkey".into())
            .spawn(move || unsafe {
                // 先建消息队列：拿到线程 id 之后 `notify_reload` 投递的消息才不会丢。
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_NOREMOVE);
                THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);

                apply_wanted(&app);

                loop {
                    let result = GetMessageW(&mut msg, None, 0, 0);
                    if result.0 <= 0 {
                        break;
                    }
                    match msg.message {
                        WM_HOTKEY => {
                            let main_app = app.clone();
                            // 窗口创建 / 显隐必须在主线程执行。
                            if let Err(err) = app.run_on_main_thread(move || {
                                if let Err(err) = crate::hub::toggle(&main_app) {
                                    log::warn!("切换组合窗口失败：{err}");
                                }
                            }) {
                                // 投递失败 = 这一次按键被丢掉（事件循环还没起来等），必须留痕，
                                // 否则会和「注册失败」一样，用户按了没反应而无从排查。
                                log::warn!("投递组合窗口切换任务失败：{err}");
                            }
                        }
                        WM_RELOAD => {
                            // 用户换了组合键：给它一套全新的重试额度。
                            RETRY_LEFT.store(RETRY_ATTEMPTS, Ordering::SeqCst);
                            apply_wanted(&app);
                        }
                        WM_RETRY => apply_wanted(&app),
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
        notify(WM_RELOAD);
    }

    /// 向快捷键线程投递一条消息（线程未启动则忽略）。
    fn notify(message: u32) {
        let thread_id = THREAD_ID.load(Ordering::SeqCst);
        if thread_id == 0 {
            return;
        }
        if let Err(err) = unsafe {
            PostThreadMessageW(thread_id, message, WPARAM(0), LPARAM(0))
        } {
            log::warn!("通知快捷键线程失败：{err}");
        }
    }

    /// 按记录的组合键重新注册：先注销旧组合，再注册新组合，并把结果记进状态。
    unsafe fn apply_wanted(app: &AppHandle) {
        let _ = UnregisterHotKey(None, HOTKEY_ID);
        let wanted = lock().shortcut.clone();

        if wanted.is_empty() {
            RETRY_LEFT.store(0, Ordering::SeqCst);
            publish(
                app,
                HotkeyStatus {
                    shortcut: wanted,
                    registered: false,
                    error: None,
                },
            );
            return;
        }

        let Some((modifiers, key)) = parse(&wanted) else {
            // 组合键本身写错（配置文件被手改）：重试没有意义。
            RETRY_LEFT.store(0, Ordering::SeqCst);
            publish(
                app,
                HotkeyStatus {
                    error: Some(format!("无法解析快捷键「{wanted}」")),
                    shortcut: wanted,
                    registered: false,
                },
            );
            return;
        };

        match RegisterHotKey(None, HOTKEY_ID, modifiers, key) {
            Ok(()) => {
                RETRY_LEFT.store(RETRY_ATTEMPTS, Ordering::SeqCst);
                publish(
                    app,
                    HotkeyStatus {
                        shortcut: wanted,
                        registered: true,
                        error: None,
                    },
                );
            }
            Err(err) => {
                // 拿到重试额度前先把结果记下来：即便最后重试成功，设置页也能看到
                // 「曾经没抢到」这件事的最终结论（重试成功会再推一次）。
                publish(
                    app,
                    HotkeyStatus {
                        shortcut: wanted,
                        registered: false,
                        error: Some(err.to_string()),
                    },
                );
                schedule_retry();
            }
        }
    }

    /// 状态变化时记日志并广播给前端（没变化就不打扰）。
    fn publish(app: &AppHandle, next: HotkeyStatus) {
        let changed = {
            let mut status = lock();
            let changed = status.shortcut != next.shortcut
                || status.registered != next.registered
                || status.error != next.error;
            *status = next.clone();
            changed
        };
        if !changed {
            return;
        }

        match (next.registered, next.error.as_deref()) {
            (true, _) => log::info!("全局快捷键 {} 已生效", next.shortcut),
            (false, Some(err)) => log::warn!("全局快捷键 {} 未生效：{err}", next.shortcut),
            (false, None) => log::info!("未注册全局快捷键（已清除）"),
        }

        if let Err(err) = app.emit(STATUS_EVENT, &next) {
            log::warn!("广播快捷键状态失败：{err}");
        }
    }

    /// 注册失败后延迟重投一次注册请求。
    ///
    /// 覆盖「上一次进程还没退干净、系统还没释放这个组合键」这种启动期竞态：它通常
    /// 几百毫秒内自愈。额度用完就放弃——如果组合键是被别的程序长期占着，一直重试
    /// 等于和对方抢，不如明确告诉用户换个键（状态里带着系统给的原因）。
    fn schedule_retry() {
        let left = RETRY_LEFT.load(Ordering::SeqCst);
        if left == 0 {
            return;
        }
        RETRY_LEFT.store(left - 1, Ordering::SeqCst);
        // 只为睡一秒后再投一条消息：用独立线程而不是 `SetTimer`，省掉一套
        // 定时器句柄的创建 / 销毁与对应的 Win32 类型。
        let _ = std::thread::Builder::new()
            .name("prism3-hotkey-retry".into())
            .spawn(|| {
                std::thread::sleep(RETRY_INTERVAL);
                notify(WM_RETRY);
            });
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