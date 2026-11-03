//! 进程列表与结束进程（监控页的「进程」卡片）。
//!
//! 与 `monitor.rs` 的硬件/负载采集分开：进程枚举走独立的 `sysinfo::System`，
//! 只在监控页轮询时刷新，不增加常驻开销。结束进程按 PID 定位，并拦下
//! **本应用自身、系统关键进程**，避免前端误操作把系统或应用本身杀掉。

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::error::{AppError, AppResult};

/// 进程列表排序维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessSort {
    Cpu,
    Memory,
}

/// 一条进程信息（序列化给监控页）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    /// 占整机 CPU 的百分比（0-100，已按逻辑核心数归一化）。
    pub cpu_usage: f32,
    /// 常驻内存（字节）。
    pub memory: u64,
    /// 可执行文件路径（读不到时为空）。
    pub exe: String,
    /// 是否受保护（本应用或系统关键进程，前端禁用结束按钮）。
    pub protected: bool,
}

/// 由 Tauri 托管的进程采集状态。
pub struct ProcessState {
    system: Mutex<System>,
    cpu_count: f32,
    self_pid: u32,
}

impl ProcessState {
    pub fn new() -> Self {
        let mut system = System::new();
        // 先建立 CPU 基线：进程的 `cpu_usage()` 需要两次采样的差值。
        system.refresh_cpu_all();
        let cpu_count = system.cpus().len().max(1) as f32;
        Self {
            system: Mutex::new(system),
            cpu_count,
            self_pid: std::process::id(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, System> {
        self.system
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ProcessState {
    fn default() -> Self {
        Self::new()
    }
}

/// 系统关键进程：结束它们会直接导致蓝屏 / 注销 / 无法登录。
const PROTECTED_NAMES: &[&str] = &[
    "system",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "services.exe",
    "lsass.exe",
    "winlogon.exe",
    "fontdrvhost.exe",
    "dwm.exe",
];

fn is_protected(state: &ProcessState, pid: u32, name: &str) -> bool {
    pid <= 4
        || pid == state.self_pid
        || PROTECTED_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

/// 刷新一遍进程并返回按指定维度排序的前 `limit` 个。
pub fn list(state: &ProcessState, sort: ProcessSort, limit: usize) -> Vec<ProcessInfo> {
    let mut system = state.lock();
    system.refresh_processes(ProcessesToUpdate::All, true);

    let mut processes: Vec<ProcessInfo> = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let pid = pid.as_u32();
            let name = process.name().to_string_lossy().into_owned();
            ProcessInfo {
                pid,
                cpu_usage: (process.cpu_usage() / state.cpu_count).clamp(0.0, 100.0),
                memory: process.memory(),
                exe: process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                protected: is_protected(state, pid, &name),
                name,
            }
        })
        .collect();

    match sort {
        ProcessSort::Cpu => {
            processes.sort_by(|left, right| right.cpu_usage.total_cmp(&left.cpu_usage));
        }
        ProcessSort::Memory => {
            processes.sort_by_key(|process| std::cmp::Reverse(process.memory));
        }
    }
    processes.truncate(limit.clamp(1, 60));
    processes
}

/// 结束一个进程；本应用自身与系统关键进程一律拒绝。
pub fn kill(state: &ProcessState, pid: u32) -> AppResult<()> {
    if pid <= 4 || pid == state.self_pid {
        return Err(AppError::Other("该进程受保护，不能结束".into()));
    }

    let mut system = state.lock();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let Some(process) = system.process(Pid::from_u32(pid)) else {
        return Err(AppError::Other("进程不存在或已结束".into()));
    };
    let name = process.name().to_string_lossy();
    if is_protected(state, pid, &name) {
        return Err(AppError::Other("该进程受保护，不能结束".into()));
    }
    if process.kill() {
        log::info!("结束进程 pid={pid} name={name}");
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "结束进程失败（可能需要管理员权限）：{name}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_critical_processes_are_protected() {
        let state = ProcessState::new();
        assert!(is_protected(&state, 4, "System"));
        assert!(is_protected(&state, state.self_pid, "prism3.exe"));
        assert!(is_protected(&state, 500, "lsass.exe"));
        assert!(!is_protected(&state, 500, "notepad.exe"));
    }

    #[test]
    fn list_is_sorted_and_limited() {
        let state = ProcessState::new();
        let by_memory = list(&state, ProcessSort::Memory, 3);
        assert!(by_memory.len() <= 3);
        assert!(by_memory
            .windows(2)
            .all(|pair| pair[0].memory >= pair[1].memory));
        assert!(by_memory.iter().all(|process| process.cpu_usage <= 100.0));
    }
}
