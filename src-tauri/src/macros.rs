//! 宏：录制全局鼠标点击与键盘操作，并按步骤回放；步骤也可在前端拖拽编排。
//!
//! 录制用 Windows 低级钩子（`WH_KEYBOARD_LL` / `WH_MOUSE_LL`）：钩子只在本应用
//! 正在录制 / 播放时安装，平时不占系统资源。录制中按 **F9** 结束；播放中按**任意键**
//! 立即停止（回放注入的键鼠事件带 `INJECTED` 标记，不会被当成用户按键）。
//!
//! 鼠标与键盘都记录按下 / 抬起：抬起步骤的 `delay_ms` 是距离上一个操作的实际等待
//! （中间没有别的操作时，它就等于按住时长）；事件按发生顺序入列表，因此
//! 「按住 W → 中途点击鼠标 → 再松开」这类并发操作能原样回放。
//! 鼠标移动轨迹、滚轮、长按产生的自动重复字符不入步骤列表（连续文字请用「文本」步骤）。
//!
//! 宏存在 `%APPDATA%\<identifier>\macros.json`（与 `config.json` 同目录、同样采用
//! 临时文件 + rename 原子替换）。回放使用前端当前编辑中的步骤，不强制先保存。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_notification::NotificationExt;

use crate::error::{AppError, AppResult};

/// 宏列表文件名。
const MACROS_FILE: &str = "macros.json";
/// 单个宏最多保存多少步：足够覆盖几分钟的键鼠操作，又不会把 JSON 撑得过大。
const MAX_STEPS: usize = 5000;
/// 整份列表的宏数量上限（只防误操作，正常使用远低于它）。
const MAX_MACROS: usize = 200;
/// 单步文本最多多少字符。
const MAX_TEXT_CHARS: usize = 4000;
/// 录制时两步之间的等待上限：用户中途去泡咖啡，回放时不该真的等五分钟。
const RECORD_DELAY_CAP_MS: u64 = 3000;
/// 手动「等待」步骤的上限（10 分钟）。
const MANUAL_DELAY_CAP_MS: u64 = 600_000;
/// 录制结束键 F9 的虚拟键码。
const STOP_KEY_VK: u32 = 0x78;

const MOD_CTRL: u8 = 0b0001;
const MOD_ALT: u8 = 0b0010;
const MOD_SHIFT: u8 = 0b0100;
const MOD_WIN: u8 = 0b1000;

/// 一个宏。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MacroEntry {
    pub id: String,
    pub name: String,
    pub steps: Vec<MacroStep>,
    /// 秒级 unix 时间戳（与项目 / 笔记同口径）。
    pub created_at: u64,
    pub updated_at: u64,
}

/// 一步操作：`delay_ms` 是执行该动作**之前**的等待。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MacroStep {
    pub delay_ms: u64,
    pub action: MacroAction,
}

/// 步骤动作。序列化后带 `type` 标签（`key` / `keyDown` / `keyUp` / `mouseClick` /
/// `mouseDown` / `mouseUp` / `text` / `wait`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MacroAction {
    /// 一次性按键（按下 + 抬起），用于手工编排快捷键。
    Key {
        /// 形如 `Ctrl+Shift+A`；可用 `Enter` / `F5` / `slash` 等命名键。
        combo: String,
    },
    /// 录制出的物理按键按下事件；`key` 是单个键名（修饰键也单独成步）。
    KeyDown {
        key: String,
    },
    /// 与 `KeyDown` 配对的抬起事件；`delay_ms` 是松开前的等待（无其他事件时即按住时长）。
    KeyUp {
        key: String,
    },
    /// 一次性鼠标点击（移动 + 按下 + 抬起），用于手工编排。
    MouseClick {
        x: i32,
        y: i32,
        #[serde(default)]
        button: MouseButton,
    },
    /// 录制出的鼠标按下事件（只按下，不抬起，可与按住键盘并发）。
    MouseDown {
        x: i32,
        y: i32,
        #[serde(default)]
        button: MouseButton,
    },
    /// 与 `MouseDown` 配对的抬起事件；`delay_ms` 是松开前的等待（无其他事件时即按住时长）。
    MouseUp {
        #[serde(default)]
        button: MouseButton,
    },
    Text {
        text: String,
    },
    Wait {
        ms: u64,
    },
}

impl Default for MacroAction {
    fn default() -> Self {
        Self::Wait { ms: 0 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// 录制结束时返回给前端的草稿。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroRecording {
    pub steps: Vec<MacroStep>,
    /// 无法映射成回放按键的事件数（如多媒体键、部分输入法键）。
    pub skipped: usize,
    /// 本次录制是否退到了「轮询兼容模式」（低级钩子被系统回收或被游戏拦截）。
    pub compat: bool,
}

/// 录制 / 回放的全局状态（前端轮询用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroState {
    pub recording: bool,
    pub recorded_steps: usize,
    pub playing: bool,
    /// 是否有一段已经结束、但还没被 `stop_macro_recording` 取走的录制
    /// （例如在别的页面按了 F9）。前端据此补一次「取回」。
    pub pending_recording: bool,
    /// 发起录制的窗口 label（`main` / `hub`）。只有这个窗口的前端页面负责
    /// 取回录制，避免主窗口与组合窗口同时抢同一段结果。
    pub origin_label: Option<String>,
    /// 当前录制是否已退到轮询兼容模式（钩子被回收 / 被反作弊拦截）。
    pub recording_compat: bool,
}

/// 宏列表（Tauri 托管的唯一数据源）。
pub struct MacroStore {
    macros: RwLock<Vec<MacroEntry>>,
}

impl MacroStore {
    pub fn new(macros: Vec<MacroEntry>) -> Self {
        Self {
            macros: RwLock::new(macros),
        }
    }

    fn snapshot(&self) -> Vec<MacroEntry> {
        self.macros
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// 全局录制 / 播放状态。录制细节（缓冲区、钩子线程句柄）在 Windows 的 `imp` 模块里。
static RECORDING_ACTIVE: AtomicBool = AtomicBool::new(false);
static RECORDED_STEPS: AtomicUsize = AtomicUsize::new(0);
static PLAYING: AtomicBool = AtomicBool::new(false);
static RECORDING_ORIGIN_LABEL: Mutex<Option<String>> = Mutex::new(None);
/// 录制兜底：低级钩子被系统静默回收（回调超时）或被游戏反作弊拦截时，
/// 录制线程里的看门狗会把它切到「轮询 `GetAsyncKeyState`」的兼容模式。
static COMPAT_RECORDING: AtomicBool = AtomicBool::new(false);

/// 发起录制 / 回放的窗口。任务结束时由后台线程把它重新显示出来。
#[derive(Clone)]
struct Origin {
    app: AppHandle,
    label: String,
}

impl Origin {
    fn reveal(self) {
        let Origin { app, label } = self;
        let handle = app.clone();
        let result = app.run_on_main_thread(move || {
            let Some(window) = handle.get_webview_window(&label) else {
                return;
            };
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        });
        if let Err(err) = result {
            log::warn!("重新显示宏操作窗口失败：{err}");
        }
    }
}

fn lock_origin_label() -> std::sync::MutexGuard<'static, Option<String>> {
    RECORDING_ORIGIN_LABEL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn macros_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_config_dir()?.join(MACROS_FILE))
}

/// 启动时读取宏列表。文件不存在 / 损坏都回退为空列表，不阻断应用启动。
pub fn load(app: &AppHandle) -> Vec<MacroEntry> {
    let Ok(path) = macros_path(app) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_else(|err| {
        log::warn!("读取宏列表失败 path={} err={err}", path.display());
        Vec::new()
    })
}

/// 原子写入宏列表（临时文件 + rename，避免写一半损坏原文件）。
fn write_macros(app: &AppHandle, macros: &[MacroEntry]) -> AppResult<()> {
    let path = macros_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(macros)?)?;
    std::fs::rename(&tmp, &path)?;
    log::debug!("宏列表已保存 path={}", path.display());
    Ok(())
}

fn now_secs() -> u64 {
    crate::db::now_secs()
}

fn new_macro_id() -> String {
    use std::sync::atomic::AtomicU64;
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    format!(
        "macro-{millis}-{}",
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// 校验并规范化用户提交的宏列表。
///
/// 对「手写 / 前端传上来的」数据做最后一道关：空 id 补上、空名补默认、延迟与文本
/// 长度截到上限、按键组合必须能解析。录制产生的步骤本身已经满足这些约束。
fn normalize_macros(mut macros: Vec<MacroEntry>) -> AppResult<Vec<MacroEntry>> {
    if macros.len() > MAX_MACROS {
        return Err(AppError::Other(format!("宏数量不能超过 {MAX_MACROS} 个")));
    }
    let now = now_secs();
    for entry in &mut macros {
        if entry.id.trim().is_empty() {
            entry.id = new_macro_id();
        }
        entry.name = entry.name.trim().to_string();
        if entry.name.is_empty() {
            entry.name = "未命名宏".to_string();
        }
        entry.steps = normalize_steps(std::mem::take(&mut entry.steps))?;
        if entry.created_at == 0 {
            entry.created_at = now;
        }
        entry.updated_at = now;
    }

    let mut ids = std::collections::HashSet::new();
    for entry in &macros {
        if !ids.insert(entry.id.clone()) {
            return Err(AppError::Other(format!("宏 id 重复：{}", entry.id)));
        }
    }
    Ok(macros)
}

fn normalize_steps(steps: Vec<MacroStep>) -> AppResult<Vec<MacroStep>> {
    if steps.len() > MAX_STEPS {
        return Err(AppError::Other(format!("单个宏最多 {MAX_STEPS} 步")));
    }
    let mut normalized = Vec::with_capacity(steps.len());
    for (index, mut step) in steps.into_iter().enumerate() {
        let position = index + 1;
        step.delay_ms = step.delay_ms.min(MANUAL_DELAY_CAP_MS);
        match &mut step.action {
            MacroAction::MouseClick { .. } | MacroAction::MouseDown { .. } => {}
            MacroAction::MouseUp { .. } => {}
            MacroAction::Key { combo } => {
                let trimmed = combo.trim().to_string();
                if parse_combo(&trimmed).is_none() {
                    return Err(AppError::Other(format!(
                        "第 {position} 步按键格式不支持：{trimmed}"
                    )));
                }
                *combo = trimmed;
            }
            MacroAction::KeyDown { key } | MacroAction::KeyUp { key } => {
                let trimmed = key.trim().to_string();
                if single_key_vk(&trimmed).is_none() {
                    return Err(AppError::Other(format!(
                        "第 {position} 步按键名不支持：{trimmed}"
                    )));
                }
                *key = trimmed;
            }
            MacroAction::Text { text } => {
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(AppError::Other(format!(
                        "第 {position} 步文本超过 {MAX_TEXT_CHARS} 字"
                    )));
                }
            }
            MacroAction::Wait { ms } => {
                *ms = (*ms).min(MANUAL_DELAY_CAP_MS);
            }
        }
        normalized.push(step);
    }
    Ok(normalized)
}

// ------------------------------ 命令 ------------------------------

/// 读取全部宏（窗口挂载时拉一次）。
#[tauri::command]
pub fn get_macros(state: State<'_, MacroStore>) -> Vec<MacroEntry> {
    state.snapshot()
}

/// 整份保存宏列表（增删改都走这一个命令，顺序即前端列表顺序）。
#[tauri::command]
pub fn save_macros(
    app: AppHandle,
    state: State<'_, MacroStore>,
    macros: Vec<MacroEntry>,
) -> AppResult<()> {
    let normalized = normalize_macros(macros)?;
    write_macros(&app, &normalized)?;
    *state
        .macros
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = normalized;
    Ok(())
}

/// 当前是否在录制 / 播放，以及已录制的步数（前端每 500ms 轮询一次）。
#[tauri::command]
pub fn get_macro_state() -> MacroState {
    MacroState {
        recording: RECORDING_ACTIVE.load(Ordering::SeqCst),
        recorded_steps: RECORDED_STEPS.load(Ordering::SeqCst),
        playing: PLAYING.load(Ordering::SeqCst),
        pending_recording: imp::has_pending_recording(),
        origin_label: lock_origin_label().clone(),
        recording_compat: COMPAT_RECORDING.load(Ordering::SeqCst),
    }
}

/// 开始全局录制（鼠标按下 + 键盘按下）。录制中按 F9 结束。
///
/// 发起窗口会立刻隐藏（主窗口 / 组合窗口都适用），结束时由后台线程重新显示，
/// 并把「谁发起的录制」写进状态，避免两个窗口同时抢同一段录制结果。
#[tauri::command(async)]
pub fn start_macro_recording(app: AppHandle, window: WebviewWindow) -> AppResult<()> {
    if PLAYING.load(Ordering::SeqCst) {
        return Err(AppError::Other("正在播放宏，先停止再录制".into()));
    }
    if RECORDING_ACTIVE.load(Ordering::SeqCst) {
        return Err(AppError::Other("已经在录制了".into()));
    }

    let origin = Origin {
        app: app.clone(),
        label: window.label().to_string(),
    };
    // 每次录制都从「钩子模式」开始；看门狗确认钩子失效后才会切到轮询。
    COMPAT_RECORDING.store(false, Ordering::SeqCst);
    // 失败时把旧的 origin 还回去：可能上一段录制还没取回，那个归属不能被本次尝试覆盖。
    let previous_label = lock_origin_label().clone();
    *lock_origin_label() = Some(origin.label.clone());

    if let Err(err) = imp::start_recording(Some(origin.clone())) {
        *lock_origin_label() = previous_label;
        return Err(err);
    }

    if let Err(err) = window.hide() {
        // 隐藏失败不影响录制本身：界面仍然可见，用户也能点停止。
        log::warn!("开始录制时隐藏窗口失败：{err}");
    }

    // 窗口已经藏起来了，用系统通知告诉用户「怎么结束」。
    if let Err(err) = app
        .notification()
        .builder()
        .title("Prism3 宏录制")
        .body("正在记录鼠标与键盘。按 F9 结束，窗口会自动回来。")
        .show()
    {
        log::debug!("发送录制提示通知失败：{err}");
    }

    Ok(())
}

/// 结束录制并返回录到的步骤。F9 已经提前结束时，这里负责把缓冲区取回来。
#[tauri::command(async)]
pub fn stop_macro_recording() -> AppResult<MacroRecording> {
    let result = imp::stop_recording();
    *lock_origin_label() = None;
    result
}

/// 回放一组步骤。立即返回；实际执行在后台线程，播放状态用 `get_macro_state` 查询。
///
/// 组合窗口铺满整屏且置顶，回放前必须先把它藏起来，否则鼠标点击会打到蒙版上；
/// 播放结束后再自动显示回来。主窗口不隐藏（3 秒倒计时足够切到目标窗口）。
#[tauri::command(async)]
pub fn play_macro(app: AppHandle, window: WebviewWindow, steps: Vec<MacroStep>) -> AppResult<()> {
    if RECORDING_ACTIVE.load(Ordering::SeqCst) {
        return Err(AppError::Other("正在录制，先结束录制再播放".into()));
    }
    if imp::has_pending_recording() {
        return Err(AppError::Other(
            "上一段录制还没取回，先结束录制并保存".into(),
        ));
    }
    if PLAYING.load(Ordering::SeqCst) {
        return Err(AppError::Other("已经有宏在播放".into()));
    }
    let steps = normalize_steps(steps)?;
    if steps.is_empty() {
        return Err(AppError::Other("宏里还没有步骤".into()));
    }

    let from_hub = window.label() == crate::hub::HUB_LABEL;
    let origin = from_hub.then(|| Origin {
        app,
        label: window.label().to_string(),
    });

    if from_hub {
        if let Err(err) = window.hide() {
            return Err(AppError::Other(format!("隐藏组合窗口失败：{err}")));
        }
    }

    match imp::start_playback(steps, origin.clone()) {
        Ok(()) => Ok(()),
        Err(err) => {
            // 回放没起来时不能让组合窗口一直藏着。
            if let Some(origin) = origin {
                origin.reveal();
            }
            Err(err)
        }
    }
}

/// 停止回放（幂等）。F9 / 任意真实按键也会触发同一个停止路径。
#[tauri::command(async)]
pub fn stop_macro_playback() -> AppResult<()> {
    imp::stop_playback()
}

// ------------------------------ 按键组合解析 ------------------------------

/// 把 `Ctrl+Shift+A` 解析成 `(修饰位, 虚拟键码)`。
///
/// 解析规则与 `hotkey.rs` 的全局快捷键一致，并补齐了录制会遇到的 F1-F24、方向键、
/// 标点、小键盘等命名；解析失败返回 `None`（保存时会被拒绝，避免“录下来了却放不出来”）。
fn parse_combo(input: &str) -> Option<(u8, u32)> {
    let mut modifiers = 0u8;
    let mut key = None;

    for token in input
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
    {
        match token.as_str() {
            "ctrl" | "control" => modifiers |= MOD_CTRL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "meta" | "super" | "cmd" => modifiers |= MOD_WIN,
            other => {
                if key.is_some() {
                    return None;
                }
                key = Some(key_vk(other)?);
            }
        }
    }

    key.map(|key| (modifiers, key))
}

/// 单个物理键名（允许修饰键本身，供录制出的按下 / 抬起事件使用）。
fn single_key_vk(name: &str) -> Option<u32> {
    key_vk(&name.trim().to_ascii_lowercase())
}

/// 命名键 → 虚拟键码。字母 / 数字 / 标点也接受单个字符形式（如 `Ctrl+/`）。
fn key_vk(token: &str) -> Option<u32> {
    let mut chars = token.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ch.is_ascii_alphanumeric() {
            return Some(ch.to_ascii_uppercase() as u32);
        }
        if let Some(code) = punctuation_vk(ch) {
            return Some(code);
        }
    }

    if let Some(rest) = token.strip_prefix('f') {
        if let Ok(number) = rest.parse::<u32>() {
            if (1..=24).contains(&number) {
                return Some(0x70 + number - 1);
            }
        }
    }
    if let Some(rest) = token.strip_prefix("num") {
        if let Ok(number) = rest.parse::<u32>() {
            if (0..=9).contains(&number) {
                return Some(0x60 + number);
            }
        }
    }

    let code = match token {
        "ctrl" | "control" => 0x11,
        "alt" | "menu" => 0x12,
        "shift" => 0x10,
        "win" | "meta" | "super" | "cmd" => 0x5B,
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
        "comma" => 0xBC,
        "period" => 0xBE,
        "slash" => 0xBF,
        "semicolon" => 0xBA,
        "quote" => 0xDE,
        "backtick" => 0xC0,
        "minus" => 0xBD,
        "plus" | "equal" => 0xBB,
        "bracketleft" => 0xDB,
        "backslash" => 0xDC,
        "bracketright" => 0xDD,
        "nummultiply" => 0x6A,
        "numadd" => 0x6B,
        "numseparator" => 0x6C,
        "numsubtract" => 0x6D,
        "numdecimal" => 0x6E,
        "numdivide" => 0x6F,
        _ => return None,
    };
    Some(code)
}

fn punctuation_vk(ch: char) -> Option<u32> {
    let code = match ch {
        ',' => 0xBC,
        '.' => 0xBE,
        '/' => 0xBF,
        ';' => 0xBA,
        '\'' => 0xDE,
        '`' => 0xC0,
        '-' => 0xBD,
        '+' | '=' => 0xBB,
        '[' => 0xDB,
        '\\' => 0xDC,
        ']' => 0xDD,
        _ => return None,
    };
    Some(code)
}

/// 虚拟键码 → 规范化名称（录制时写入 `combo`）。
fn key_label(vk: u32) -> Option<&'static str> {
    let label = match vk {
        0x10 | 0xA0 | 0xA1 => "Shift",
        0x11 | 0xA2 | 0xA3 => "Ctrl",
        0x12 | 0xA4 | 0xA5 => "Alt",
        0x5B | 0x5C => "Win",
        0x41..=0x5A => return Some(ALPHABET[(vk - 0x41) as usize]),
        0x30..=0x39 => return Some(DIGITS[(vk - 0x30) as usize]),
        0x70..=0x87 => return Some(F_KEYS[(vk - 0x70) as usize]),
        0x60..=0x69 => return Some(NUMPAD[(vk - 0x60) as usize]),
        0x20 => "space",
        0x0D => "enter",
        0x09 => "tab",
        0x1B => "esc",
        0x08 => "backspace",
        0x2E => "delete",
        0x2D => "insert",
        0x24 => "home",
        0x23 => "end",
        0x21 => "pageup",
        0x22 => "pagedown",
        0x26 => "up",
        0x28 => "down",
        0x25 => "left",
        0x27 => "right",
        0xBC => "comma",
        0xBE => "period",
        0xBF => "slash",
        0xBA => "semicolon",
        0xDE => "quote",
        0xC0 => "backtick",
        0xBD => "minus",
        0xBB => "plus",
        0xDB => "bracketleft",
        0xDC => "backslash",
        0xDD => "bracketright",
        0x6A => "numMultiply",
        0x6B => "numAdd",
        0x6C => "numSeparator",
        0x6D => "numSubtract",
        0x6E => "numDecimal",
        0x6F => "numDivide",
        _ => return None,
    };
    Some(label)
}

const ALPHABET: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];
const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const F_KEYS: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];
const NUMPAD: [&str; 10] = [
    "num0", "num1", "num2", "num3", "num4", "num5", "num6", "num7", "num8", "num9",
];

/// 一步录制的结果：延迟 + 动作。
///
/// `cap_delay = true` 时把两步间的等待截到 `RECORD_DELAY_CAP_MS`（用户中途走开）；
/// 抬起事件（`KeyUp` / `MouseUp`）不截断，它的延迟是距离上一个事件的实际等待
/// （简单的长按步骤里就等于按住时长）。
fn recorded_step(
    steps: &[MacroStep],
    last: std::time::Instant,
    action: MacroAction,
    cap_delay: bool,
) -> MacroStep {
    let delay_ms = if steps.is_empty() {
        0
    } else {
        let elapsed = last.elapsed().as_millis() as u64;
        if cap_delay {
            elapsed.min(RECORD_DELAY_CAP_MS)
        } else {
            elapsed.min(MANUAL_DELAY_CAP_MS)
        }
    };
    MacroStep { delay_ms, action }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashSet;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicU32, AtomicU64};
    use std::sync::mpsc;
    use std::sync::{Mutex, MutexGuard};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::Threading::{
        GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetLastInputInfo, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
        KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, LASTINPUTINFO,
        MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_CONTROL,
        VK_LBUTTON, VK_LWIN, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, GetMessageW, GetSystemMetrics, PeekMessageW, PostQuitMessage,
        PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
        LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL, WH_MOUSE_LL,
        WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    };

    use super::{
        key_label, parse_combo, recorded_step, single_key_vk, MacroAction, MacroRecording,
        MacroStep, MouseButton, COMPAT_RECORDING, MAX_STEPS, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN,
        PLAYING, RECORDED_STEPS, RECORDING_ACTIVE, STOP_KEY_VK,
    };

    struct RecordBuffer {
        steps: Vec<MacroStep>,
        last: Instant,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win: bool,
        /// 当前按住的主键：屏蔽长按产生的自动重复。
        pressed: std::collections::HashSet<u32>,
        skipped: usize,
    }

    impl RecordBuffer {
        fn new() -> Self {
            Self {
                steps: Vec::new(),
                last: Instant::now(),
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
                pressed: std::collections::HashSet::new(),
                skipped: 0,
            }
        }

        fn push(&mut self, action: MacroAction, cap_delay: bool) {
            let step = recorded_step(&self.steps, self.last, action, cap_delay);
            self.last = Instant::now();
            self.steps.push(step);
            RECORDED_STEPS.store(self.steps.len(), std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// 录制缓冲区与钩子线程句柄只在 Windows 上存在。
    static RECORD_BUFFER: Mutex<Option<RecordBuffer>> = Mutex::new(None);
    static RECORDING_TID: AtomicU32 = AtomicU32::new(0);
    static RECORDING_HANDLE: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static PLAYBACK_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static PLAYBACK_HANDLE: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    /// 钩子回调被调用的次数（含移动、注入等最终没记成步骤的事件）。
    /// 看门狗用它判断「系统确实有输入、钩子却一次都没回调」。
    static HOOK_EVENTS: AtomicU64 = AtomicU64::new(0);
    /// 看门狗要求录制线程重新安装钩子的线程消息。
    const WM_APP_REHOOK: u32 = WM_APP + 1;

    fn lock_recording() -> MutexGuard<'static, Option<RecordBuffer>> {
        RECORD_BUFFER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_handle(
        handle: &'static Mutex<Option<JoinHandle<()>>>,
    ) -> MutexGuard<'static, Option<JoinHandle<()>>> {
        handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn start_recording(mut origin: Option<super::Origin>) -> super::AppResult<()> {
        if lock_recording().is_some() {
            return Err(super::AppError::Other(
                "上一次录制还没有保存，先把步骤取回来".into(),
            ));
        }

        RECORDED_STEPS.store(0, std::sync::atomic::Ordering::SeqCst);
        RECORDING_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        HOOK_EVENTS.store(0, std::sync::atomic::Ordering::SeqCst);
        COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
        *lock_recording() = Some(RecordBuffer::new());

        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("prism3-macro-record".into())
            .spawn(move || {
                let mut msg = MSG::default();
                // 先建消息队列，随后写入的线程 id 才能收到跨线程消息。
                let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                let tid = unsafe { GetCurrentThreadId() };
                RECORDING_TID.store(tid, std::sync::atomic::Ordering::SeqCst);
                // 游戏 / 反作弊 / 高负载下普通优先级线程会被饿死；低级钩子回调
                // 超过 LowLevelHooksTimeout（默认 300ms）累计 10 次，Windows 就会
                // 静默把钩子摘掉。先把录制线程提到最高优先级，排除这条路径。
                let _ =
                    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };

                let (mut keyboard, mut mouse) = match install_hooks() {
                    Ok((keyboard, mouse)) => (Some(keyboard), Some(mouse)),
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        RECORDING_TID.store(0, std::sync::atomic::Ordering::SeqCst);
                        RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                        COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut watchdog = spawn_watchdog(tid);

                loop {
                    let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                    if result.0 <= 0 {
                        break;
                    }
                    if msg.message == WM_APP_REHOOK {
                        // 钩子句柄必须由装它的线程重装：先摘旧的，再装新的。
                        if let Some(hook) = mouse.take() {
                            let _ = unsafe { UnhookWindowsHookEx(hook) };
                        }
                        if let Some(hook) = keyboard.take() {
                            let _ = unsafe { UnhookWindowsHookEx(hook) };
                        }
                        match install_hooks() {
                            Ok((next_keyboard, next_mouse)) => {
                                keyboard = Some(next_keyboard);
                                mouse = Some(next_mouse);
                                log::info!("宏录制钩子已重新安装");
                            }
                            Err(err) => {
                                log::error!("重新安装宏录制钩子失败：{err}");
                                break;
                            }
                        }
                    }
                }

                // 先停看门狗再摘钩子：它只会往本线程投消息，但留着运行会污染下一段录制。
                RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Some(watchdog) = watchdog.take() {
                    let _ = watchdog.join();
                }
                if let Some(hook) = mouse.take() {
                    let _ = unsafe { UnhookWindowsHookEx(hook) };
                }
                if let Some(hook) = keyboard.take() {
                    let _ = unsafe { UnhookWindowsHookEx(hook) };
                }
                COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
                RECORDING_TID.store(0, std::sync::atomic::Ordering::SeqCst);
                // 窗口在开始录制时被藏起来了：无论 F9 还是停止按钮，结束都把它放回来。
                if let Some(origin) = origin.take() {
                    origin.reveal();
                }
            });

        let handle = match handle {
            Ok(handle) => handle,
            Err(err) => {
                RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                *lock_recording() = None;
                return Err(super::AppError::Other(format!("启动录制线程失败：{err}")));
            }
        };

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {
                *lock_handle(&RECORDING_HANDLE) = Some(handle);
                log::info!("宏录制已开始（F9 结束）");
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = handle.join();
                RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                *lock_recording() = None;
                Err(super::AppError::Other(format!("安装键鼠钩子失败：{err}")))
            }
            Err(_) => {
                RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = unsafe {
                    PostThreadMessageW(
                        RECORDING_TID.load(std::sync::atomic::Ordering::SeqCst),
                        WM_QUIT,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                let _ = handle.join();
                *lock_recording() = None;
                Err(super::AppError::Other("启动录制超时".into()))
            }
        }
    }

    pub fn has_pending_recording() -> bool {
        lock_recording().is_some()
    }

    pub fn stop_recording() -> super::AppResult<MacroRecording> {
        let tid = RECORDING_TID.load(std::sync::atomic::Ordering::SeqCst);
        if tid != 0 {
            // 线程已经因 F9 自己结束时，这里失败也无所谓；join 会立刻返回。
            let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }

        let handle = lock_handle(&RECORDING_HANDLE).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }

        let buffer = lock_recording().take();
        let compat = COMPAT_RECORDING.swap(false, std::sync::atomic::Ordering::SeqCst);
        RECORDED_STEPS.store(0, std::sync::atomic::Ordering::SeqCst);
        RECORDING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);

        match buffer {
            Some(buffer) => {
                log::info!(
                    "宏录制结束：{} 步，跳过 {} 个事件，兼容模式={compat}",
                    buffer.steps.len(),
                    buffer.skipped
                );
                Ok(MacroRecording {
                    steps: buffer.steps,
                    skipped: buffer.skipped,
                    compat,
                })
            }
            None => Err(super::AppError::Other("当前没有在录制".into())),
        }
    }

    fn key_down(vk: u32) -> bool {
        unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 }
    }

    /// 兼容轮询专用：是否有修饰键正被按住（用于「裸 F9 结束录制」）。
    fn modifiers_down() -> bool {
        [
            0x10u32, 0xA0, 0xA1, 0x11, 0xA2, 0xA3, 0x12, 0xA4, 0xA5, 0x5B, 0x5C,
        ]
        .iter()
        .any(|vk| key_down(*vk))
    }

    /// 系统「最近一次输入」的毫秒 tick。看门狗靠它区分「用户没动」和
    /// 「用户动了、钩子却没收到」。
    fn last_input_tick() -> u32 {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            info.dwTime
        } else {
            0
        }
    }

    fn install_hooks() -> Result<(HHOOK, HHOOK), String> {
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) };
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) };
        match (keyboard, mouse) {
            (Ok(keyboard), Ok(mouse)) => Ok((keyboard, mouse)),
            (keyboard, mouse) => {
                let keyboard_error = keyboard.as_ref().err().map(|err| err.to_string());
                let mouse_error = mouse.as_ref().err().map(|err| err.to_string());
                if let Ok(hook) = keyboard {
                    let _ = unsafe { UnhookWindowsHookEx(hook) };
                }
                if let Ok(hook) = mouse {
                    let _ = unsafe { UnhookWindowsHookEx(hook) };
                }
                Err(keyboard_error
                    .or(mouse_error)
                    .unwrap_or_else(|| "未知错误".to_string()))
            }
        }
    }

    fn push_recorded(action: MacroAction, cap_delay: bool) {
        if let Some(buffer) = lock_recording().as_mut() {
            buffer.push(action, cap_delay);
        }
    }

    /// 钩子彻底失效时的兜底：每 4ms 轮询 `GetAsyncKeyState` + `GetCursorPos`，
    /// 把按下 / 抬起补成同样的步骤。
    ///
    /// 轮询没有 `LowLevelHooksTimeout`，也不依赖钩子链：游戏反作弊屏蔽低级钩子、
    /// 或钩子被系统回收后，只有这条路仍可能录到输入。开始轮询时已经按住的键
    /// 只记为「基线」，不补按下、松开时也不补抬起，避免凭空多出半截步骤。
    fn poll_input(tid: u32) {
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
        // 只轮询能变成步骤的键（约 100 个），避免每 4ms 扫 0x01..0xFF 的无效查询。
        // 泛修饰键（VK_SHIFT / VK_CONTROL / VK_MENU）在按住左右键时也会报 down，
        // 和 0xA0..0xA5 的具体键重复，这里只留具体键，避免修饰键被记两遍。
        let polled: Vec<u32> = (0x01..=0xFF)
            .filter(|vk| !matches!(*vk, 0x10..=0x12))
            .filter(|vk| key_label(*vk).is_some())
            .collect();
        let mut held: HashSet<u32> = polled.iter().copied().filter(|vk| key_down(*vk)).collect();
        let mut initial = held.clone();
        let buttons = [
            (VK_LBUTTON.0 as u32, MouseButton::Left),
            (VK_RBUTTON.0 as u32, MouseButton::Right),
            (VK_MBUTTON.0 as u32, MouseButton::Middle),
        ];
        let mut button_held = [false; 3];
        let mut button_initial = [false; 3];
        for (index, (vk, _)) in buttons.iter().enumerate() {
            button_held[index] = key_down(*vk);
            button_initial[index] = button_held[index];
        }

        while RECORDING_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
            && COMPAT_RECORDING.load(std::sync::atomic::Ordering::SeqCst)
        {
            for vk in &polled {
                let down = key_down(*vk);
                let was = held.contains(vk);
                if down == was {
                    continue;
                }
                if down {
                    held.insert(*vk);
                    // 裸 F9 = 结束录制，和钩子路径保持一致。
                    if *vk == STOP_KEY_VK && !modifiers_down() {
                        COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
                        let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
                        return;
                    }
                    if let Some(label) = key_label(*vk) {
                        push_recorded(
                            MacroAction::KeyDown {
                                key: label.to_string(),
                            },
                            true,
                        );
                    }
                } else {
                    held.remove(vk);
                    if initial.remove(vk) {
                        continue;
                    }
                    if let Some(label) = key_label(*vk) {
                        push_recorded(
                            MacroAction::KeyUp {
                                key: label.to_string(),
                            },
                            false,
                        );
                    }
                }
            }

            // 鼠标按键：按下时记录光标坐标，抬起只记按键（和钩子路径同构）。
            for (index, (vk, button)) in buttons.iter().enumerate() {
                let down = key_down(*vk);
                if down == button_held[index] {
                    continue;
                }
                button_held[index] = down;
                if down {
                    let mut point = POINT::default();
                    if unsafe { GetCursorPos(&mut point) }.is_ok() {
                        push_recorded(
                            MacroAction::MouseDown {
                                x: point.x,
                                y: point.y,
                                button: *button,
                            },
                            true,
                        );
                    }
                } else if button_initial[index] {
                    button_initial[index] = false;
                } else {
                    push_recorded(MacroAction::MouseUp { button: *button }, false);
                }
            }

            if RECORDED_STEPS.load(std::sync::atomic::Ordering::SeqCst) >= MAX_STEPS {
                COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
                return;
            }
            thread::sleep(Duration::from_millis(4));
        }
        COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn start_polling_fallback(tid: u32) {
        if COMPAT_RECORDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        log::warn!("宏录制：钩子收不到输入，切换轮询兼容模式（游戏 / 反作弊可能拦截了低级钩子）");
        let spawned = thread::Builder::new()
            .name("prism3-macro-poll".into())
            .spawn(move || poll_input(tid));
        if let Err(err) = spawned {
            log::error!("启动轮询兼容录制失败：{err}");
            COMPAT_RECORDING.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// 看门狗：每秒对比「系统最近输入时间」与「钩子回调计数」。
    ///
    /// Windows 会在低级钩子回调累计超时 10 次后静默摘钩（不会通知进程），
    /// 普通桌面场景几乎碰不到，但游戏满载 / 反作弊环境下很常见。有输入、
    /// 计数不涨就先要求重装；重装后仍然如此就切到轮询兜底。
    fn spawn_watchdog(tid: u32) -> Option<JoinHandle<()>> {
        let handle = thread::Builder::new()
            .name("prism3-macro-watchdog".into())
            .spawn(move || {
                let mut last_tick = last_input_tick();
                let mut last_events = HOOK_EVENTS.load(std::sync::atomic::Ordering::SeqCst);
                let mut rehooked = false;
                let mut since_sample = Instant::now();
                while RECORDING_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(200));
                    if since_sample.elapsed() < Duration::from_secs(1) {
                        continue;
                    }
                    since_sample = Instant::now();
                    if !RECORDING_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    let tick = last_input_tick();
                    let events = HOOK_EVENTS.load(std::sync::atomic::Ordering::SeqCst);
                    let input_advanced = tick != last_tick;
                    last_tick = tick;
                    if input_advanced && events == last_events {
                        if !rehooked {
                            rehooked = true;
                            log::warn!("宏录制：系统有输入、钩子却没回调，尝试重新安装钩子");
                            let _ = unsafe {
                                PostThreadMessageW(tid, WM_APP_REHOOK, WPARAM(0), LPARAM(0))
                            };
                            // 重装要几毫秒才生效：从下一刻重新观察，避免把
                            // 重装窗口里的输入误判成「装了也没回调」。
                            last_tick = last_input_tick();
                            last_events = HOOK_EVENTS.load(std::sync::atomic::Ordering::SeqCst);
                            continue;
                        }
                        if !COMPAT_RECORDING.load(std::sync::atomic::Ordering::SeqCst) {
                            start_polling_fallback(tid);
                        }
                    } else if events != last_events {
                        // 钩子活着：下次再掉线允许先重装一次。
                        rehooked = false;
                    }
                    last_events = events;
                }
            });
        match handle {
            Ok(handle) => Some(handle),
            Err(err) => {
                log::warn!("启动宏录制看门狗失败：{err}");
                None
            }
        }
    }

    fn modifier_index(vk: u32) -> Option<usize> {
        match vk {
            0x10 | 0xA0 | 0xA1 => Some(0),
            0x11 | 0xA2 | 0xA3 => Some(1),
            0x12 | 0xA4 | 0xA5 => Some(2),
            0x5B | 0x5C => Some(3),
            _ => None,
        }
    }

    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        // 回调计数只给看门狗判断「钩子是否还活着」用，不参与步骤生成。
        HOOK_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !RECORDING_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let data = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // 回放注入的按键不算用户操作（录制与回放不会同时进行，这里是兜底）。
        if data.flags.contains(LLKHF_INJECTED) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let vk = data.vkCode;
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let mut guard = lock_recording();
        let Some(buffer) = guard.as_mut() else {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        };

        // 裸 F9 = 结束录制；带修饰键的 F9 仍按普通按键记录，留给用户组合。
        if down && vk == STOP_KEY_VK && !buffer.ctrl && !buffer.alt && !buffer.shift && !buffer.win
        {
            drop(guard);
            unsafe { PostQuitMessage(0) };
            return LRESULT(1);
        }

        // 修饰键也按物理按下 / 抬起记录：这样「按住 Ctrl 再按多个键」
        // 和「按住 Shift 时点鼠标」都能原样还原，而不是折叠成一次性组合键。
        if let Some(index) = modifier_index(vk) {
            let slot = match index {
                0 => &mut buffer.shift,
                1 => &mut buffer.ctrl,
                2 => &mut buffer.alt,
                _ => &mut buffer.win,
            };
            *slot = down;
        }

        if down {
            // 长按产生的重复 keydown 只记第一次；按住时长由随后的 KeyUp 记录。
            if !buffer.pressed.insert(vk) {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }
            match key_label(vk) {
                Some(label) => {
                    buffer.push(
                        MacroAction::KeyDown {
                            key: label.to_string(),
                        },
                        true,
                    );
                    if buffer.steps.len() >= MAX_STEPS {
                        drop(guard);
                        unsafe { PostQuitMessage(0) };
                    }
                }
                None => buffer.skipped += 1,
            }
        } else {
            buffer.pressed.remove(&vk);
            // 抬起事件的延迟 = 按住时长，不做 3 秒截断。
            if let Some(label) = key_label(vk) {
                buffer.push(
                    MacroAction::KeyUp {
                        key: label.to_string(),
                    },
                    false,
                );
            }
        }

        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        HOOK_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !RECORDING_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if data.flags & LLMHF_INJECTED != 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        // 按下与抬起都记录：抬起步骤的延迟就是鼠标按住时长，
        // 并且可以和键盘按住事件交错，回放时能形成真正的并发按键 / 鼠标状态。
        let button = match wparam.0 as u32 {
            WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
            WM_LBUTTONUP => Some((MouseButton::Left, false)),
            WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
            WM_RBUTTONUP => Some((MouseButton::Right, false)),
            WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
            WM_MBUTTONUP => Some((MouseButton::Middle, false)),
            _ => None,
        };
        if let Some((button, down)) = button {
            if let Some(buffer) = lock_recording().as_mut() {
                if down {
                    buffer.push(
                        MacroAction::MouseDown {
                            x: data.pt.x,
                            y: data.pt.y,
                            button,
                        },
                        true,
                    );
                } else {
                    buffer.push(MacroAction::MouseUp { button }, false);
                }
            }
        }

        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub fn start_playback(
        steps: Vec<MacroStep>,
        origin: Option<super::Origin>,
    ) -> super::AppResult<()> {
        PLAYBACK_STOP.store(false, std::sync::atomic::Ordering::SeqCst);
        PLAYING.store(true, std::sync::atomic::Ordering::SeqCst);

        // 先占住上一个已结束线程的句柄（正常时已经结束，join 立即返回）。
        if let Some(previous) = lock_handle(&PLAYBACK_HANDLE).take() {
            let _ = previous.join();
        }

        let (hook_tid, hook_handle) = match spawn_stop_hook() {
            Ok(value) => value,
            Err(err) => {
                PLAYING.store(false, std::sync::atomic::Ordering::SeqCst);
                return Err(err);
            }
        };

        // 回放线程正常结束 / 启动失败的兜底路径都要关掉停止钩子线程，
        // 因此把 (线程 id, 句柄) 放进共享槽位，由实际拿到的一方负责 join。
        let hook_session = std::sync::Arc::new(Mutex::new(Some((hook_tid, hook_handle))));
        let session_in_thread = hook_session.clone();
        let origin_for_error = origin.clone();
        let handle = thread::Builder::new()
            .name("prism3-macro-playback".into())
            .spawn(move || {
                run_playback(&steps);
                let session = session_in_thread
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some((tid, handle)) = session {
                    stop_hook_thread(tid, handle);
                }
                PLAYING.store(false, std::sync::atomic::Ordering::SeqCst);
                // 组合窗口在回放前被藏起来了：播放结束（自然结束或被按键打断）后放回来。
                if let Some(origin) = origin {
                    origin.reveal();
                }
            });

        match handle {
            Ok(handle) => {
                *lock_handle(&PLAYBACK_HANDLE) = Some(handle);
                log::info!("宏回放已开始（按任意键停止）");
                Ok(())
            }
            Err(err) => {
                let session = hook_session
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if let Some((tid, handle)) = session {
                    stop_hook_thread(tid, handle);
                }
                PLAYING.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Some(origin) = origin_for_error {
                    origin.reveal();
                }
                Err(super::AppError::Other(format!("启动回放线程失败：{err}")))
            }
        }
    }

    pub fn stop_playback() -> super::AppResult<()> {
        PLAYBACK_STOP.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = lock_handle(&PLAYBACK_HANDLE).take() {
            let _ = handle.join();
        }
        PLAYING.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    /// 播放期间的「任意键停止」钩子：必须在自己的消息循环线程里安装。
    unsafe extern "system" fn playback_stop_hook(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 {
            let data = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if down && !data.flags.contains(LLKHF_INJECTED) {
                PLAYBACK_STOP.store(true, std::sync::atomic::Ordering::SeqCst);
                unsafe { PostQuitMessage(0) };
                // 吞掉这次真实按键，避免它同时落到目标窗口里。
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn spawn_stop_hook() -> super::AppResult<(u32, JoinHandle<()>)> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("prism3-macro-stop-hook".into())
            .spawn(move || {
                let mut msg = MSG::default();
                let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                let tid = unsafe { GetCurrentThreadId() };
                // 与录制同理：回放期间游戏占用 CPU 时，低优先级线程的钩子回调
                // 超时也会被系统摘掉，这里同样提到最高优先级。
                let _ =
                    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
                match unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(playback_stop_hook), None, 0)
                } {
                    Ok(hook) => {
                        let _ = ready_tx.send(Ok(tid));
                        loop {
                            let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                            if result.0 <= 0 {
                                break;
                            }
                        }
                        let _ = unsafe { UnhookWindowsHookEx(hook) };
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                    }
                }
            })
            .map_err(|err| super::AppError::Other(format!("启动停止钩子失败：{err}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(tid)) => Ok((tid, handle)),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(super::AppError::Other(format!("安装停止钩子失败：{err}")))
            }
            Err(_) => {
                let _ = handle.join();
                Err(super::AppError::Other("启动停止钩子超时".into()))
            }
        }
    }

    fn stop_hook_thread(tid: u32, handle: JoinHandle<()>) {
        if tid != 0 {
            let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        let _ = handle.join();
    }

    /// 回放过程中仍按住的键 / 鼠标键。中途停止或出错时必须把它们全部松开，
    /// 否则会留下一个「一直按着 Shift」或「鼠标左键卡住」的全局状态。
    #[derive(Default)]
    struct HeldState {
        keys: Vec<u32>,
        buttons: Vec<MouseButton>,
    }

    impl HeldState {
        fn press_key(&mut self, vk: u32) {
            if !self.keys.contains(&vk) {
                self.keys.push(vk);
            }
        }

        fn release_key(&mut self, vk: u32) {
            self.keys.retain(|pressed| *pressed != vk);
        }

        fn press_button(&mut self, button: MouseButton) {
            if !self.buttons.contains(&button) {
                self.buttons.push(button);
            }
        }

        fn release_button(&mut self, button: MouseButton) {
            self.buttons.retain(|pressed| *pressed != button);
        }

        fn release_all(&mut self) {
            for vk in self.keys.drain(..).rev() {
                let _ = send_inputs(&[key_input(vk, KEYEVENTF_KEYUP)]);
            }
            for button in self.buttons.drain(..).rev() {
                let _ = mouse_button_event(button, false);
            }
        }
    }

    fn run_playback(steps: &[MacroStep]) {
        let mut held = HeldState::default();
        for step in steps {
            if !wait_interruptible(step.delay_ms) {
                break;
            }
            if !execute_action(&step.action, &mut held) {
                break;
            }
        }
        // 长按步骤可能中途按下还没松开：退出前统一释放。
        held.release_all();
    }

    fn wait_interruptible(ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if PLAYBACK_STOP.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            // 10ms 一个检查点：停止请求最多延迟 10ms 生效，又不会忙等。
            thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }

    fn execute_action(action: &MacroAction, held: &mut HeldState) -> bool {
        if PLAYBACK_STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        match action {
            MacroAction::MouseClick { x, y, button } => click_at(*x, *y, *button),
            MacroAction::MouseDown { x, y, button } => {
                if !move_cursor_to(*x, *y) {
                    return false;
                }
                if PLAYBACK_STOP.load(std::sync::atomic::Ordering::SeqCst) {
                    return false;
                }
                if !mouse_button_event(*button, true) {
                    return false;
                }
                held.press_button(*button);
                true
            }
            MacroAction::MouseUp { button } => {
                held.release_button(*button);
                mouse_button_event(*button, false)
            }
            MacroAction::Key { combo } => match parse_combo(combo) {
                Some((modifiers, key)) => send_combo(modifiers, key),
                None => {
                    log::warn!("回放遇到无法解析的按键：{combo}");
                    false
                }
            },
            MacroAction::KeyDown { key } => match single_key_vk(key) {
                Some(vk) => {
                    if !send_inputs(&[key_input(vk, KEYBD_EVENT_FLAGS(0))]) {
                        return false;
                    }
                    held.press_key(vk);
                    true
                }
                None => {
                    log::warn!("回放遇到无法解析的按键：{key}");
                    false
                }
            },
            MacroAction::KeyUp { key } => match single_key_vk(key) {
                Some(vk) => {
                    held.release_key(vk);
                    send_inputs(&[key_input(vk, KEYEVENTF_KEYUP)])
                }
                None => {
                    log::warn!("回放遇到无法解析的按键：{key}");
                    false
                }
            },
            MacroAction::Text { text } => send_text(text),
            MacroAction::Wait { ms } => wait_interruptible(*ms),
        }
    }

    fn move_cursor_to(x: i32, y: i32) -> bool {
        let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(2);
        let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(2);

        // 多显示器 + 缩放：按虚拟桌面归一化到 0..65535，比 SetCursorPos 更稳。
        let nx = (((x - vx) as f64 * 65535.0 / (vw - 1) as f64).round()).clamp(0.0, 65535.0) as i32;
        let ny = (((y - vy) as f64 * 65535.0 / (vh - 1) as f64).round()).clamp(0.0, 65535.0) as i32;

        let move_input = mouse_input(
            nx,
            ny,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        );
        if !send_inputs(&[move_input]) {
            return false;
        }
        thread::sleep(Duration::from_millis(15));
        true
    }

    fn mouse_button_event(button: MouseButton, down: bool) -> bool {
        let flags = match (button, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        send_inputs(&[mouse_input(0, 0, flags)])
    }

    fn click_at(x: i32, y: i32, button: MouseButton) -> bool {
        if !move_cursor_to(x, y) {
            return false;
        }
        if PLAYBACK_STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        if !mouse_button_event(button, true) {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
        // 即使此刻收到停止请求，也必须补发抬起，避免鼠标键卡在按下状态。
        let _ = mouse_button_event(button, false);
        true
    }

    fn send_combo(modifiers: u8, key: u32) -> bool {
        let mut down = Vec::with_capacity(5);
        if modifiers & MOD_CTRL != 0 {
            down.push(key_input(VK_CONTROL.0 as u32, KEYBD_EVENT_FLAGS(0)));
        }
        if modifiers & MOD_ALT != 0 {
            down.push(key_input(VK_MENU.0 as u32, KEYBD_EVENT_FLAGS(0)));
        }
        if modifiers & MOD_SHIFT != 0 {
            down.push(key_input(VK_SHIFT.0 as u32, KEYBD_EVENT_FLAGS(0)));
        }
        if modifiers & MOD_WIN != 0 {
            down.push(key_input(VK_LWIN.0 as u32, KEYBD_EVENT_FLAGS(0)));
        }
        down.push(key_input(key, KEYBD_EVENT_FLAGS(0)));

        if !send_inputs(&down) {
            // 批量发送失败时退化为空操作；SendInput 失败通常一个键都没进去。
            return false;
        }
        // 给目标窗口一点时间处理按下 / 抬起；太短时快捷键常被漏掉。
        thread::sleep(Duration::from_millis(20));

        let mut up = Vec::with_capacity(down.len());
        up.push(key_input(key, KEYEVENTF_KEYUP));
        if modifiers & MOD_WIN != 0 {
            up.push(key_input(VK_LWIN.0 as u32, KEYEVENTF_KEYUP));
        }
        if modifiers & MOD_SHIFT != 0 {
            up.push(key_input(VK_SHIFT.0 as u32, KEYEVENTF_KEYUP));
        }
        if modifiers & MOD_ALT != 0 {
            up.push(key_input(VK_MENU.0 as u32, KEYEVENTF_KEYUP));
        }
        if modifiers & MOD_CTRL != 0 {
            up.push(key_input(VK_CONTROL.0 as u32, KEYEVENTF_KEYUP));
        }
        send_inputs(&up)
    }

    fn send_text(text: &str) -> bool {
        for unit in text.encode_utf16() {
            if PLAYBACK_STOP.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            // 换行统一发回车：直接发 Unicode 换行在不少控件里不会换行。
            if unit == b'\n' as u16 {
                if !send_inputs(&[key_input(0x0D, KEYBD_EVENT_FLAGS(0))]) {
                    return false;
                }
                thread::sleep(Duration::from_millis(5));
                let _ = send_inputs(&[key_input(0x0D, KEYEVENTF_KEYUP)]);
            } else {
                if unit == b'\r' as u16 {
                    continue;
                }
                let down = key_input_unicode(unit, KEYBD_EVENT_FLAGS(0));
                let up = key_input_unicode(unit, KEYEVENTF_KEYUP);
                // Unicode 键必须成对发：先按下再抬起。
                if !send_inputs(&[down, up]) {
                    return false;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        true
    }

    fn key_input(vk: u32, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk as u16),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn key_input_unicode(unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: flags | KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_inputs(inputs: &[INPUT]) -> bool {
        if inputs.is_empty() {
            return true;
        }
        let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            log::warn!("发送键鼠事件失败：期望 {} 条，成功 {sent} 条", inputs.len());
            return false;
        }
        true
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{AppError, AppResult, MacroRecording, MacroStep};

    pub fn start_recording(_origin: Option<super::Origin>) -> AppResult<()> {
        Err(AppError::Other("宏录制仅支持 Windows".into()))
    }

    pub fn has_pending_recording() -> bool {
        false
    }

    pub fn stop_recording() -> AppResult<MacroRecording> {
        Err(AppError::Other("宏录制仅支持 Windows".into()))
    }

    pub fn start_playback(_steps: Vec<MacroStep>, _origin: Option<super::Origin>) -> AppResult<()> {
        Err(AppError::Other("宏回放仅支持 Windows".into()))
    }

    pub fn stop_playback() -> AppResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_parser_accepts_modifiers_named_keys_and_punctuation() {
        assert_eq!(
            parse_combo("Ctrl+Shift+A"),
            Some((MOD_CTRL | MOD_SHIFT, 'A' as u32))
        );
        assert_eq!(parse_combo("ctrl+alt+f9"), Some((MOD_CTRL | MOD_ALT, 0x78)));
        assert_eq!(parse_combo("Win+Enter"), Some((MOD_WIN, 0x0D)));
        assert_eq!(parse_combo("Ctrl+/"), Some((MOD_CTRL, 0xBF)));
        assert_eq!(parse_combo("Shift+1"), Some((MOD_SHIFT, '1' as u32)));
        assert_eq!(parse_combo("num3"), Some((0, 0x63)));
        assert_eq!(parse_combo("F24"), Some((0, 0x87)));
        assert_eq!(parse_combo("A+B"), None, "两个主键应被拒绝");
        assert_eq!(parse_combo("Ctrl"), None, "没有主键应被拒绝");
        assert_eq!(parse_combo("Ctrl+Unsupported"), None);
    }

    #[test]
    fn key_labels_round_trip_through_the_parser() {
        for label in [
            "space",
            "enter",
            "tab",
            "esc",
            "backspace",
            "delete",
            "insert",
            "home",
            "end",
            "pageup",
            "pagedown",
            "up",
            "down",
            "left",
            "right",
            "comma",
            "period",
            "slash",
            "semicolon",
            "quote",
            "backtick",
            "minus",
            "plus",
            "bracketleft",
            "backslash",
            "bracketright",
            "num0",
            "num9",
            "F1",
            "F24",
        ] {
            assert!(parse_combo(label).is_some(), "命名键 {label} 应可解析");
        }
        for vk in [0x41u32, 0x5A, 0x30, 0x39] {
            assert!(key_label(vk).is_some());
        }
        // 录制出的按住 / 松开事件允许修饰键单独成步。
        assert_eq!(single_key_vk("Ctrl"), Some(0x11));
        assert_eq!(single_key_vk("Shift"), Some(0x10));
        assert_eq!(single_key_vk("Alt"), Some(0x12));
        assert_eq!(single_key_vk("Win"), Some(0x5B));
        assert_eq!(single_key_vk("S"), Some(0x53));
        assert_eq!(single_key_vk("Nope"), None);
    }

    #[test]
    fn macros_are_normalized_before_saving() {
        let macros = normalize_macros(vec![MacroEntry {
            id: String::new(),
            name: "  ".into(),
            steps: vec![
                MacroStep {
                    delay_ms: u64::MAX,
                    action: MacroAction::Wait { ms: u64::MAX },
                },
                MacroStep {
                    delay_ms: 0,
                    action: MacroAction::Key {
                        combo: " Ctrl+S ".into(),
                    },
                },
            ],
            created_at: 0,
            updated_at: 0,
        }])
        .expect("应能规范化");

        assert!(!macros[0].id.is_empty());
        assert_eq!(macros[0].name, "未命名宏");
        assert_eq!(macros[0].steps[0].delay_ms, MANUAL_DELAY_CAP_MS);
        assert_eq!(
            macros[0].steps[0].action,
            MacroAction::Wait {
                ms: MANUAL_DELAY_CAP_MS
            }
        );
        assert_eq!(
            macros[0].steps[1].action,
            MacroAction::Key {
                combo: "Ctrl+S".into()
            }
        );
        assert!(macros[0].created_at > 0);
        assert!(macros[0].updated_at > 0);
    }

    #[test]
    fn invalid_macro_steps_are_rejected() {
        let invalid_key = normalize_macros(vec![MacroEntry {
            steps: vec![MacroStep {
                delay_ms: 0,
                action: MacroAction::Key {
                    combo: "Nope".into(),
                },
            }],
            ..MacroEntry::default()
        }]);
        assert!(invalid_key.is_err());

        let duplicate = normalize_macros(vec![
            MacroEntry {
                id: "same".into(),
                ..MacroEntry::default()
            },
            MacroEntry {
                id: "same".into(),
                ..MacroEntry::default()
            },
        ]);
        assert!(duplicate.is_err());
    }

    #[test]
    fn recorded_delay_is_zero_for_the_first_step_and_capped_afterwards() {
        let first = recorded_step(
            &[],
            std::time::Instant::now(),
            MacroAction::Wait { ms: 0 },
            true,
        );
        assert_eq!(first.delay_ms, 0);

        let last = std::time::Instant::now() - std::time::Duration::from_secs(10);
        let capped = recorded_step(
            std::slice::from_ref(&first),
            last,
            MacroAction::Wait { ms: 0 },
            true,
        );
        assert_eq!(capped.delay_ms, RECORD_DELAY_CAP_MS);

        // 抬起事件不截断：它的延迟就是真实的按住时长。
        let held = recorded_step(
            std::slice::from_ref(&first),
            last,
            MacroAction::KeyUp { key: "S".into() },
            false,
        );
        assert_eq!(held.delay_ms, 10_000);
    }

    #[test]
    fn advanced_input_actions_serialize_and_validate() {
        let actions = [
            MacroAction::KeyDown { key: "S".into() },
            MacroAction::KeyUp { key: "S".into() },
            MacroAction::MouseDown {
                x: 100,
                y: 200,
                button: MouseButton::Left,
            },
            MacroAction::MouseUp {
                button: MouseButton::Left,
            },
        ];
        let steps: Vec<MacroStep> = actions
            .iter()
            .cloned()
            .map(|action| MacroStep {
                delay_ms: 0,
                action,
            })
            .collect();
        let normalized = normalize_steps(steps).expect("高级输入动作应可保存");

        let value = serde_json::to_value(&normalized[0]).expect("应可序列化");
        assert_eq!(value["action"]["type"], "keyDown");
        assert_eq!(value["action"]["key"], "S");

        let value = serde_json::to_value(&normalized[3]).expect("应可序列化");
        assert_eq!(value["action"]["type"], "mouseUp");
        assert_eq!(value["action"]["button"], "left");

        // 按住事件的延迟不被 3 秒上限截断（这里是 5 秒）。
        let mut steps = normalized;
        steps[1].delay_ms = 5_000;
        let normalized = normalize_steps(steps).expect("按住时长应保留");
        assert_eq!(normalized[1].delay_ms, 5_000);

        let bad = normalize_steps(vec![MacroStep {
            delay_ms: 0,
            action: MacroAction::KeyDown {
                key: "Ctrl+S".into(),
            },
        }]);
        assert!(bad.is_err(), "按下 / 抬起只接受单个物理键名");
    }

    #[test]
    fn compat_polling_covers_keyboard_and_skips_mouse_buttons() {
        // 轮询只扫能映射成步骤的键；常见键与 F9（结束键）都必须覆盖。
        for vk in [
            0x41u32, 0x30, 0x20, 0x0D, 0x10, 0x11, 0x12, 0x5B, 0x25, 0x26, 0x27, 0x28, 0xBC, 0x60,
            0x70, 0x78,
        ] {
            assert!(key_label(vk).is_some(), "轮询应覆盖键 {vk:#x}");
        }
        // 鼠标按键走单独分支；不排除的话会被当成未知键静默丢掉。
        for vk in [0x01u32, 0x02, 0x04] {
            assert!(key_label(vk).is_none(), "鼠标键不应出现在键盘轮询表里");
        }
        assert!(key_label(STOP_KEY_VK).is_some(), "F9 必须能被轮询识别");
    }

    #[test]
    fn recording_results_export_the_compat_flag() {
        // 前端靠这两个字段提示「这次是兼容模式录的」，序列化名字不能漂。
        let recording = MacroRecording {
            steps: Vec::new(),
            skipped: 0,
            compat: true,
        };
        assert_eq!(
            serde_json::to_value(&recording).expect("应可序列化")["compat"],
            true
        );

        let state = MacroState {
            recording: false,
            recorded_steps: 0,
            playing: false,
            pending_recording: false,
            origin_label: None,
            recording_compat: true,
        };
        assert_eq!(
            serde_json::to_value(&state).expect("应可序列化")["recordingCompat"],
            true
        );
    }
}
