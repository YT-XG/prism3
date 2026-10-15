use std::sync::RwLock;

use serde::Serialize;

use crate::monitor::SystemSnapshot;

/// 应用运行状态：灰条据此决定色相与动画（前端把档位映射到 `--qp-tone` 与动画名）。
///
/// 这是「运行状态 → 视觉」的唯一通道。未来新增业务状态（错误 / 同步中 / 离线）在此加值即可，
/// 采集方式与广播路径不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppStatus {
    /// 常态：CPU 与内存都在低位（中性灰呼吸）。
    Normal,
    /// 偏忙（暖黄呼吸）。
    Busy,
    /// 吃紧（暖红闪烁）。
    Warn,
}

/// 升档阈值：`peak`（CPU 与内存占用里较高的一方，0-100）达到该值即上行。
const BUSY_ENTER: f32 = 50.0;
const WARN_ENTER: f32 = 70.0;

/// 降档滞回带：已升档后要退出，需比升档线低这么多。
///
/// 负载卡在阈值上时若没有滞回，档位会来回翻——`warn` 一侧是闪烁，抖动尤其显眼。
const HYSTERESIS: f32 = 3.0;
const BUSY_EXIT: f32 = BUSY_ENTER - HYSTERESIS;
const WARN_EXIT: f32 = WARN_ENTER - HYSTERESIS;

/// 由 Tauri 托管的运行状态。
///
/// 记住最后一次判定的档位：既供滞回判断，也供窗口挂载时兜底拉取（事件不缓存）。
pub struct AppStatusState {
    current: RwLock<AppStatus>,
}

impl AppStatusState {
    pub fn new() -> Self {
        Self {
            current: RwLock::new(AppStatus::Normal),
        }
    }

    pub fn current(&self) -> AppStatus {
        *self
            .current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 落定新档位，返回是否发生了变化（只有变化才需要广播）。
    pub fn update(&self, next: AppStatus) -> bool {
        let mut guard = self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = *guard != next;
        *guard = next;
        changed
    }
}

impl Default for AppStatusState {
    fn default() -> Self {
        Self::new()
    }
}

/// 取 CPU 与内存占用里更高的那一个（0-100）。
///
/// 于是「有一方超线就升档」只需对单轴分档。CPU 首帧没有有效差值（`None`）时按 0 处理，
/// 让内存单独定档。
pub fn peak_percent(snapshot: &SystemSnapshot) -> f32 {
    let cpu = snapshot.cpu_usage.unwrap_or(0.0);
    let memory = if snapshot.mem_total > 0 {
        snapshot.mem_used as f32 / snapshot.mem_total as f32 * 100.0
    } else {
        0.0
    };
    cpu.max(memory)
}

/// 按 `peak` 与上一档位判定新档位（带滞回）。
///
/// 升档线是硬的（常态 → 偏忙看 50、偏忙 → 吃紧看 70）；降档要走滞回带，
/// 因此从吃紧一路降回常态最多需要两次采样。
pub fn classify(peak: f32, previous: AppStatus) -> AppStatus {
    match previous {
        AppStatus::Warn => {
            if peak >= WARN_EXIT {
                AppStatus::Warn
            } else {
                // 已低于降档线：退回按「偏忙」继续判，可能一次连降到常态。
                classify(peak, AppStatus::Busy)
            }
        }
        AppStatus::Busy => {
            if peak >= WARN_ENTER {
                AppStatus::Warn
            } else if peak <= BUSY_EXIT {
                AppStatus::Normal
            } else {
                AppStatus::Busy
            }
        }
        AppStatus::Normal => {
            if peak >= WARN_ENTER {
                AppStatus::Warn
            } else if peak > BUSY_ENTER {
                AppStatus::Busy
            } else {
                AppStatus::Normal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(cpu: Option<f32>, mem_used: u64, mem_total: u64) -> SystemSnapshot {
        SystemSnapshot {
            cpu_usage: cpu,
            mem_used,
            mem_total,
            net_rx_per_sec: 0.0,
            net_tx_per_sec: 0.0,
            uptime_secs: 0,
            disks: Vec::new(),
        }
    }

    #[test]
    fn enter_thresholds() {
        // 50 归常态、50 以上归偏忙、70 归吃紧。
        assert_eq!(classify(0.0, AppStatus::Normal), AppStatus::Normal);
        assert_eq!(classify(50.0, AppStatus::Normal), AppStatus::Normal);
        assert_eq!(classify(50.1, AppStatus::Normal), AppStatus::Busy);
        assert_eq!(classify(69.9, AppStatus::Normal), AppStatus::Busy);
        assert_eq!(classify(70.0, AppStatus::Normal), AppStatus::Warn);
        assert_eq!(classify(100.0, AppStatus::Normal), AppStatus::Warn);
    }

    #[test]
    fn hysteresis_holds_after_entering() {
        // 偏忙：要跌到 47 及以下才回常态。
        assert_eq!(classify(48.0, AppStatus::Busy), AppStatus::Busy);
        assert_eq!(classify(47.0, AppStatus::Busy), AppStatus::Normal);
        // 吃紧：要跌破 67 才降为偏忙。
        assert_eq!(classify(68.0, AppStatus::Warn), AppStatus::Warn);
        assert_eq!(classify(67.0, AppStatus::Warn), AppStatus::Warn);
        assert_eq!(classify(66.9, AppStatus::Warn), AppStatus::Busy);
    }

    #[test]
    fn warn_can_drop_two_steps() {
        // 从吃紧一路回落：一次采样即可回到常态。
        assert_eq!(classify(40.0, AppStatus::Warn), AppStatus::Normal);
        // 但落在滞回带内仍保持偏忙。
        assert_eq!(classify(60.0, AppStatus::Warn), AppStatus::Busy);
    }

    #[test]
    fn peak_takes_the_higher_side() {
        // 内存 80%、CPU 30% —— 取内存。
        assert!((peak_percent(&snapshot(Some(30.0), 12_800, 16_000)) - 80.0).abs() < 0.01);
        // CPU 95% 高于内存 10% —— 取 CPU。
        assert!((peak_percent(&snapshot(Some(95.0), 1_000, 10_000)) - 95.0).abs() < 0.01);
    }

    #[test]
    fn peak_handles_missing_cpu_and_empty_memory() {
        // CPU 首帧无差值：按 0 处理，让内存单独定档。
        assert!((peak_percent(&snapshot(None, 8_000, 16_000)) - 50.0).abs() < 0.01);
        // 内存总量读不到时不除零。
        assert_eq!(peak_percent(&snapshot(None, 0, 0)), 0.0);
    }
}
