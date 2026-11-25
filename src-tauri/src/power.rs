//! **电脑定时关机**：倒计时 + 每天定时（可选星期），仅支持关机。
//!
//! 两种模式互斥：
//! - **倒计时**交给 Windows 的 `shutdown /s /t`：即使 Prism3 退出也会照常关机，
//!   取消用 `shutdown /a`；
//! - **每天定时**由本模块的调度线程守着（按本地时间算下一次、到点调 `shutdown /s /t 15`
//!   留 15 秒提醒），因此需要 Prism3 保持运行（关闭窗口默认收进托盘）。
//!
//! 每天定时的时 / 星期落在配置里（`shutdown_time` / `shutdown_weekdays`），只由本模块的
//! 专用命令写；`set_config` 会把这两个键换回内存权威值（与 BLE 三键同一先例）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, AppState};
use crate::error::{AppError, AppResult};

/// 定时关机状态变化的事件名（主窗口「定时关机」页据此重绘）。
pub const SHUTDOWN_EVENT: &str = "shutdown:state";

/// 倒计时允许的分钟范围（1 分钟 ~ 24 小时）。
pub const MIN_MINUTES: u32 = 1;
pub const MAX_MINUTES: u32 = 24 * 60;

/// 关机模式：倒计时 / 每天定时。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShutdownMode {
    Countdown,
    Daily,
}

/// 与前端 `ShutdownState` 一一对应：命令返回值与事件载荷同构。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownState {
    /// 是否有一个正在等待执行的关机计划。
    pub active: bool,
    /// 触发关机的秒级 unix 时间戳；未激活时为 `None`。
    pub deadline_secs: Option<u64>,
    /// 当前模式；空闲时为 `None`。
    pub mode: Option<ShutdownMode>,
    /// 每天定时的 `HH:mm`；未配置为 `None`（倒计时模式下同为空）。
    pub daily_time: Option<String>,
    /// 每天定时选中的星期（0 = 周日 … 6 = 周六）；空数组 = 每天。
    pub weekdays: Vec<u32>,
}

impl ShutdownState {
    fn idle() -> Self {
        Self {
            active: false,
            deadline_secs: None,
            mode: None,
            daily_time: None,
            weekdays: Vec::new(),
        }
    }
}

/// 定时关机的内存状态：截止时刻 + 当前模式 + 每天定时的时 / 星期。
///
/// `generation` 是每天定时线程的代号：取消 / 重设时自增，旧线程看到代号变化就退出
/// （不必 join，最迟一个检查周期内自行结束）。
#[derive(Default)]
pub struct ShutdownStore {
    inner: Mutex<Inner>,
    generation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    deadline: Option<u64>,
    mode: Option<ShutdownMode>,
    daily_time: Option<String>,
    weekdays: Vec<u32>,
}

impl ShutdownStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读当前状态；截止时刻已过就视为结束（不依赖前端来清零）。
    pub fn snapshot(&self) -> ShutdownState {
        let mut inner = self.lock();
        if inner.deadline.is_some_and(|deadline| deadline <= now_secs()) {
            inner.deadline = None;
            inner.mode = None;
        }
        ShutdownState {
            active: inner.mode.is_some(),
            deadline_secs: inner.deadline,
            mode: inner.mode,
            daily_time: inner.daily_time.clone(),
            weekdays: inner.weekdays.clone(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

// ---------------------------------------------------------------------------
// 倒计时
// ---------------------------------------------------------------------------

/// 设置倒计时（分钟）。**系统命令成功之后才记账**，失败时状态保持不变。
///
/// 开倒计时会关掉每天定时（配置与线程一起），两种模式不会同时挂着。
pub fn schedule(app: &AppHandle, store: &ShutdownStore, minutes: u32) -> AppResult<ShutdownState> {
    let seconds = u64::from(clamp_minutes(minutes)) * 60;
    run_shutdown(&["/s".to_string(), "/t".to_string(), seconds.to_string()])?;

    // 系统命令成功之后才切换模式：失败时原来的每天定时保持不变。
    store.bump_generation();
    if let Err(err) = clear_daily_config(app) {
        log::warn!("清理每天定时配置失败：{err}");
    }

    let deadline = deadline_from(now_secs(), minutes);
    {
        let mut inner = store.lock();
        inner.deadline = Some(deadline);
        inner.mode = Some(ShutdownMode::Countdown);
        inner.daily_time = None;
        inner.weekdays.clear();
    }
    let state = store.snapshot();
    broadcast(app, state.clone());
    Ok(state)
}

// ---------------------------------------------------------------------------
// 每天定时
// ---------------------------------------------------------------------------

/// 保存每天定时（`HH:mm` + 星期），写配置并让调度线程接管。
pub fn set_daily(
    app: &AppHandle,
    time: &str,
    weekdays: &[u32],
) -> AppResult<ShutdownState> {
    let (hour, minute) = parse_time(time)
        .ok_or_else(|| AppError::Other("时间格式应为 HH:mm（如 23:30）".into()))?;
    let days = normalize_days(weekdays);

    {
        let state = app.state::<AppState>();
        let snapshot = {
            let mut guard = state
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.shutdown_time = format!("{hour:02}:{minute:02}");
            guard.shutdown_weekdays = days.clone();
            guard.clone()
        };
        config::save(app, &snapshot)?;
    }

    sync_daily(app);
    Ok(app.state::<ShutdownStore>().snapshot())
}

/// 把「配置里的每天定时」同步到运行时：起停调度线程并刷新状态。
///
/// 这是每天定时的**唯一同步入口**：启动、`set_config` 变更、`reset_config`、
/// 保存 / 取消命令都走它。
pub fn sync_daily(app: &AppHandle) {
    let store = app.state::<ShutdownStore>();
    let generation = store.bump_generation();

    let (time, weekdays) = {
        let state = app.state::<AppState>();
        let guard = state
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (guard.shutdown_time.clone(), guard.shutdown_weekdays.clone())
    };

    let parsed = if time.trim().is_empty() {
        None
    } else {
        parse_time(&time)
    };

    let Some((hour, minute)) = parsed else {
        if !time.trim().is_empty() {
            log::warn!("每天定时关机的时间无效：{time:?}");
        }
        // 关掉每天定时：若正处在 15 秒提醒窗口里，撤掉那次关机。
        let mode = store.lock().mode;
        if mode == Some(ShutdownMode::Daily) {
            let _ = run_shutdown(&["/a".to_string()]);
        }
        {
            let mut inner = store.lock();
            inner.deadline = None;
            inner.mode = None;
            inner.daily_time = None;
            inner.weekdays.clear();
        }
        broadcast(app, store.snapshot());
        return;
    };

    let days = normalize_days(&weekdays);
    let normalized = format!("{hour:02}:{minute:02}");
    let target = hour * 3600 + minute * 60;

    // 每天定时接管：把在跑的倒计时从系统里撤掉。
    let mode = store.lock().mode;
    if mode == Some(ShutdownMode::Countdown) {
        let _ = run_shutdown(&["/a".to_string()]);
    }

    match local_now() {
        Some((now, weekday, secs_day)) => {
            let next = next_daily_epoch(now, weekday, secs_day, target, &days);
            let mut inner = store.lock();
            inner.deadline = Some(next);
            inner.mode = Some(ShutdownMode::Daily);
            inner.daily_time = Some(normalized);
            inner.weekdays = days.clone();
        }
        None => {
            log::warn!("每天定时关机：读取本地时间失败，先只保存计划");
            let mut inner = store.lock();
            inner.deadline = None;
            inner.mode = None;
            inner.daily_time = Some(normalized);
            inner.weekdays = days.clone();
        }
    }
    broadcast(app, store.snapshot());

    let handle = app.clone();
    std::thread::spawn(move || daily_loop(handle, generation, target, days));
}

/// 每天定时的调度线程：算下一次 → 等到点 → 交给系统关机（留 15 秒提醒）→ 排下一次。
fn daily_loop(app: AppHandle, generation: u64, target: u32, days: Vec<u32>) {
    loop {
        let store = app.state::<ShutdownStore>();
        if !store.is_current(generation) {
            return;
        }

        let Some((now, weekday, secs_day)) = local_now() else {
            log::warn!("每天定时关机：读取本地时间失败，60 秒后重试");
            if !sleep_generation(&store, generation, 60) {
                return;
            }
            continue;
        };

        let fire_at = next_daily_epoch(now, weekday, secs_day, target, &days);
        {
            let mut inner = store.lock();
            inner.deadline = Some(fire_at);
            inner.mode = Some(ShutdownMode::Daily);
        }
        broadcast(&app, store.snapshot());
        log::info!("每天定时关机：下一次在 {fire_at}（unix 秒）");

        if !wait_until(&store, generation, fire_at) {
            return;
        }
        if !store.is_current(generation) {
            return;
        }

        // 到点：交给系统执行，留 15 秒提醒；期间用户仍可在界面取消（会 `shutdown /a`）。
        match run_shutdown(&[
            "/s".to_string(),
            "/t".to_string(),
            "15".to_string(),
        ]) {
            Ok(()) => {
                {
                    let mut inner = store.lock();
                    inner.deadline = Some(now_secs() + 15);
                    inner.mode = Some(ShutdownMode::Daily);
                }
                broadcast(&app, store.snapshot());
            }
            Err(err) => log::warn!("每天定时关机执行失败：{err}"),
        }

        // 提醒窗口过去后如果机器还在（失败 / 用户撤销），继续排下一次。
        if !sleep_generation(&store, generation, 20) {
            return;
        }
    }
}

/// 取消关机计划（倒计时与每天定时都适用），并清掉配置里的每天定时。
pub fn cancel(app: &AppHandle, store: &ShutdownStore) -> AppResult<ShutdownState> {
    store.bump_generation();

    let had_plan = store.lock().mode.is_some();
    {
        let mut inner = store.lock();
        *inner = Inner::default();
    }
    if had_plan {
        if let Err(err) = run_shutdown(&["/a".to_string()]) {
            log::debug!("取消关机计划时系统返回：{err}");
        }
    }

    clear_daily_config(app)?;

    let state = ShutdownState::idle();
    broadcast(app, state.clone());
    Ok(state)
}

/// 把配置里的每天定时清空（只在确实有值时才落盘）。
fn clear_daily_config(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut guard = state
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.shutdown_time.is_empty() && guard.shutdown_weekdays.is_empty() {
            None
        } else {
            guard.shutdown_time.clear();
            guard.shutdown_weekdays.clear();
            Some(guard.clone())
        }
    };
    if let Some(config) = snapshot {
        config::save(app, &config)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 时间计算（纯函数，可单测）
// ---------------------------------------------------------------------------

fn broadcast(app: &AppHandle, state: ShutdownState) {
    if let Err(err) = app.emit(SHUTDOWN_EVENT, state) {
        log::warn!("广播定时关机状态失败：{err}");
    }
}

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

/// `HH:mm`（也接受 `H:mm`）→ `(时, 分)`；越界或格式不对返回 `None`。
pub fn parse_time(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// 星期去重 / 排序 / 只留 0–6；空数组保持空（= 每天）。
pub fn normalize_days(days: &[u32]) -> Vec<u32> {
    let mut days: Vec<u32> = days.iter().copied().filter(|day| *day < 7).collect();
    days.sort_unstable();
    days.dedup();
    days
}

fn day_allowed(weekday: u32, days: &[u32]) -> bool {
    days.is_empty() || days.contains(&weekday)
}

/// 下一次触发的 unix 秒数。
///
/// `weekday` 是本地星期（0 = 周日），`secs_day` 是当天已过的秒数，`target` 是目标
/// 时刻距午夜的秒数。只用「本地午夜 + 天数 × 86400」推算，不做月 / 年日期运算
/// （中国无夏令时，窗口内的整数天偏移是精确的）。
pub fn next_daily_epoch(
    now: u64,
    weekday: u32,
    secs_day: u32,
    target: u32,
    days: &[u32],
) -> u64 {
    let midnight = now.saturating_sub(u64::from(secs_day));
    for offset in 0..=7u64 {
        let candidate_day = ((u64::from(weekday) + offset) % 7) as u32;
        if !day_allowed(candidate_day, days) {
            continue;
        }
        let candidate = midnight + offset * 86_400 + u64::from(target);
        if candidate > now {
            return candidate;
        }
    }
    // 理论上到不了这里（offset = 7 一定在未来）；兜底留一周后。
    midnight + 7 * 86_400 + u64::from(target)
}

/// 读取本地时间：`(unix 秒, 星期 0=周日, 当天已过秒数)`。
#[cfg(windows)]
fn local_now() -> Option<(u64, u32, u32)> {
    use windows::Win32::System::SystemInformation::GetLocalTime;

    let time = unsafe { GetLocalTime() };
    let secs_day = u32::from(time.wHour) * 3600
        + u32::from(time.wMinute) * 60
        + u32::from(time.wSecond);
    Some((now_secs(), u32::from(time.wDayOfWeek), secs_day))
}

#[cfg(not(windows))]
fn local_now() -> Option<(u64, u32, u32)> {
    None
}

/// 等到 `deadline`；代号变化即返回 `false`（调用方据此退出线程）。
fn wait_until(store: &ShutdownStore, generation: u64, deadline: u64) -> bool {
    loop {
        if !store.is_current(generation) {
            return false;
        }
        let now = now_secs();
        if now >= deadline {
            return true;
        }
        let remaining = deadline - now;
        // 离得远时 20 秒醒一次，最后 30 秒按秒检查，保证取消能立刻生效。
        let step = if remaining > 30 { 20 } else { 1 };
        std::thread::sleep(Duration::from_secs(step));
    }
}

/// 睡 `secs` 秒；代号变化即返回 `false`。
fn sleep_generation(store: &ShutdownStore, generation: u64, secs: u64) -> bool {
    let mut left = secs;
    while left > 0 {
        if !store.is_current(generation) {
            return false;
        }
        let step = left.min(5);
        std::thread::sleep(Duration::from_secs(step));
        left -= step;
    }
    store.is_current(generation)
}

// ---------------------------------------------------------------------------
// 执行 shutdown.exe
// ---------------------------------------------------------------------------

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
        assert_eq!(deadline_from(0, u32::MAX), u64::from(MAX_MINUTES) * 60);
    }

    #[test]
    fn time_is_parsed_in_24_hour_clock() {
        assert_eq!(parse_time("23:30"), Some((23, 30)));
        assert_eq!(parse_time(" 7:05 "), Some((7, 5)));
        assert_eq!(parse_time("00:00"), Some((0, 0)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("12:60"), None);
        assert_eq!(parse_time("1230"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn weekdays_are_normalized() {
        assert_eq!(normalize_days(&[1, 3, 3, 9]), vec![1, 3]);
        assert_eq!(normalize_days(&[]), Vec::<u32>::new());
        assert_eq!(normalize_days(&[6, 0]), vec![0, 6]);
    }

    #[test]
    fn next_occurrence_is_today_when_time_is_still_ahead() {
        // 周三 22:00，目标 23:00 → 今天。
        let now = 1_700_000_000;
        let next = next_daily_epoch(now, 3, 22 * 3600, 23 * 3600, &[]);
        assert_eq!(next, now + 3600);
    }

    #[test]
    fn next_occurrence_rolls_to_tomorrow_when_time_has_passed() {
        // 周三 23:30，目标 23:00 → 明天。
        let now = 1_700_000_000;
        let next = next_daily_epoch(now, 3, 23 * 3600 + 1800, 23 * 3600, &[]);
        assert_eq!(next, now - 1800 + 86_400);
    }

    #[test]
    fn next_occurrence_skips_unselected_weekdays() {
        // 周三，只在周一关机 → 下周一（5 天后）。
        let now = 1_700_000_000;
        let next = next_daily_epoch(now, 3, 10 * 3600, 23 * 3600, &[1]);
        assert_eq!(next, now - 10 * 3600 + 5 * 86_400 + 23 * 3600);
    }

    #[test]
    fn selected_weekday_today_when_allowed_and_ahead() {
        // 周三 10:00，只选周三、目标 23:00 → 今天。
        let now = 1_700_000_000;
        let next = next_daily_epoch(now, 3, 10 * 3600, 23 * 3600, &[3]);
        assert_eq!(next, now + 13 * 3600);
    }

    #[test]
    fn expired_deadline_reads_as_idle() {
        let store = ShutdownStore::new();
        store.lock().deadline = Some(now_secs().saturating_sub(1));
        store.lock().mode = Some(ShutdownMode::Countdown);
        assert!(!store.snapshot().active);
        assert_eq!(store.lock().deadline, None);
    }

    #[test]
    fn future_deadline_stays_active() {
        let store = ShutdownStore::new();
        let deadline = now_secs() + 600;
        {
            let mut inner = store.lock();
            inner.deadline = Some(deadline);
            inner.mode = Some(ShutdownMode::Countdown);
        }
        let snapshot = store.snapshot();
        assert!(snapshot.active);
        assert_eq!(snapshot.deadline_secs, Some(deadline));
        assert_eq!(snapshot.mode, Some(ShutdownMode::Countdown));
    }
}
