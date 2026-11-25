//! **电脑定时关机**：倒计时交给 Windows 的 `shutdown /s /t`，本模块只记账 + 广播状态。
//!
//! 为什么不自己起一条定时线程：`shutdown /s /t N` 的倒计时由系统维护，即使 Prism3
//! 被关掉（托盘退出 / 崩溃）也会照常关机；取消用 `shutdown /a`。本模块只记住
//! 「这次倒计时的截止时刻」，供界面显示剩余时间，因此**不落盘**——重启应用后
//! 界面回到空闲，但系统里那份关机计划仍然有效（`shutdown /a` 也照常能取消它）。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};

/// 定时关机状态变化的事件名（主窗口「定时关机」页据此重绘）。
pub const SHUTDOWN_EVENT: &str = "shutdown:state";

/// 倒计时允许的分钟范围（1 分钟 ~ 24 小时）。
pub const MIN_MINUTES: u32 = 1;
pub const MAX_MINUTES: u32 = 24 * 60;

/// 与前端 `ShutdownState` 一一对应：命令返回值与事件载荷同构。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownState {
    /// 是否有一个由本应用挂起的关机倒计时。
    pub active: bool,
    /// 触发关机的秒级 unix 时间戳；未激活时为 `None`。
    pub deadline_secs: Option<u64>,
}

impl ShutdownState {
    /// 空闲态（没有倒计时）。
    const IDLE: Self = Self {
        active: false,
        deadline_secs: None,
    };
}

/// 定时关机的内存状态：只存截止时刻，关机计划本身由系统维护。
#[derive(Default)]
pub struct ShutdownStore {
    deadline: Mutex<Option<u64>>,
}

impl ShutdownStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读当前状态；截止时刻已过就视为结束（不依赖前端来清零）。
    pub fn snapshot(&self) -> ShutdownState {
        let mut guard = self.lock();
        match *guard {
            Some(deadline) if deadline > now_secs() => ShutdownState {
                active: true,
                deadline_secs: Some(deadline),
            },
            Some(_) => {
                *guard = None;
                ShutdownState::IDLE
            }
            None => ShutdownState::IDLE,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<u64>> {
        self.deadline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 设置倒计时（分钟）。**系统命令成功之后才记账**，失败时状态保持不变。
pub fn schedule(app: &AppHandle, store: &ShutdownStore, minutes: u32) -> AppResult<ShutdownState> {
    let seconds = u64::from(clamp_minutes(minutes)) * 60;
    run_shutdown(&["/s".to_string(), "/t".to_string(), seconds.to_string()])?;

    let deadline = deadline_from(now_secs(), minutes);
    *store.lock() = Some(deadline);
    let state = ShutdownState {
        active: true,
        deadline_secs: Some(deadline),
    };
    broadcast(app, state);
    Ok(state)
}

/// 取消倒计时，同时撤掉系统里挂起的关机计划。
///
/// `shutdown /a` 在没有计划时会失败（退出码 1116），那是正常结果，只记 DEBUG。
pub fn cancel(app: &AppHandle, store: &ShutdownStore) -> ShutdownState {
    if let Err(err) = run_shutdown(&["/a".to_string()]) {
        log::debug!("取消关机计划时系统返回：{err}");
    }
    *store.lock() = None;
    let state = ShutdownState::IDLE;
    broadcast(app, state);
    state
}

fn broadcast(app: &AppHandle, state: ShutdownState) {
    if let Err(err) = app.emit(SHUTDOWN_EVENT, state) {
        log::warn!("广播定时关机状态失败：{err}");
    }
}

/// 秒级 unix 时间戳（与 `db::now_secs` 同一口径）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// 分钟数夹到允许区间；界面传什么值都不会把系统命令写坏。
pub fn clamp_minutes(minutes: u32) -> u32 {
    minutes.clamp(MIN_MINUTES, MAX_MINUTES)
}

/// 由「当前时刻 + 分钟数」算出截止时刻。
pub fn deadline_from(now: u64, minutes: u32) -> u64 {
    now + u64::from(clamp_minutes(minutes)) * 60
}

/// Windows 的 `shutdown` 在「已经有计划在等」时的退出码。
#[cfg(windows)]
const ALREADY_SCHEDULED: i32 = 1190;

#[cfg(windows)]
fn run_shutdown(args: &[String]) -> AppResult<()> {
    use std::os::windows::process::CommandExt;

    /// 不创建控制台窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("shutdown")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code();
    // stderr 是系统本地化的（中文系统为 GBK），只进日志，不回给界面。
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        log::warn!("shutdown {args:?} 返回 {code:?}：{stderr}");
    }

    Err(match code {
        Some(ALREADY_SCHEDULED) => {
            AppError::Other("已经有一个关机计划在等待执行，请先取消或等它执行完".into())
        }
        Some(code) => AppError::Other(format!("关机命令执行失败（退出码 {code}）")),
        None => AppError::Other("关机命令被系统终止".into()),
    })
}

#[cfg(not(windows))]
fn run_shutdown(_args: &[String]) -> AppResult<()> {
    Err(AppError::Other("定时关机只支持 Windows".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_are_clamped_to_a_safe_range() {
        assert_eq!(clamp_minutes(0), MIN_MINUTES);
        assert_eq!(clamp_minutes(30), 30);
        assert_eq!(clamp_minutes(u32::MAX), MAX_MINUTES);
    }

    #[test]
    fn deadline_is_now_plus_minutes() {
        assert_eq!(deadline_from(1_000, 1), 1_060);
        assert_eq!(deadline_from(1_000, 90), 6_400);
        // 超界值按上限算，不会写出一个离谱的截止时刻。
        assert_eq!(deadline_from(0, u32::MAX), u64::from(MAX_MINUTES) * 60);
    }

    #[test]
    fn expired_deadline_reads_as_idle() {
        let store = ShutdownStore::new();
        *store.lock() = Some(now_secs().saturating_sub(1));
        assert_eq!(store.snapshot(), ShutdownState::IDLE);
        assert_eq!(*store.lock(), None);
    }

    #[test]
    fn future_deadline_stays_active() {
        let store = ShutdownStore::new();
        let deadline = now_secs() + 600;
        *store.lock() = Some(deadline);
        assert_eq!(
            store.snapshot(),
            ShutdownState {
                active: true,
                deadline_secs: Some(deadline)
            }
        );
    }
}
