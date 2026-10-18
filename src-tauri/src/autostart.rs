//! 开机自启：配置里的 `auto_start` 就是系统启动项的期望状态。
//!
//! 这里只有「配置 → 系统」一条同步路径（幂等）：启动时跑一次（默认配置即开），
//! 设置页改开关时再跑一次（见 `commands.rs` 的 `set_config` / `reset_config`）。
//! 落地点是 `tauri-plugin-autostart`（Windows 上即 `HKCU\...\CurrentVersion\Run`）。
//!
//! **开发模式一律不注册**：`tauri dev` 下当前可执行文件在 `target\debug` 里，
//! 把它写进启动项的结果是「开机启动一个只有在开发环境里才跑得起来的调试程序」，
//! 用户还会以为功能坏了。判定用 `cfg!(debug_assertions)`（`pnpm tauri dev` 走 dev
//! profile 必然为 true，`pnpm tauri build` 走 release profile 为 false），而不是
//! `tauri::is_dev()`——后者看的是 tauri 的 `custom-protocol` 特性，本仓库并未声明它。

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::config::AppConfig;

/// 是否是开发模式构建（见模块头：不用 `tauri::is_dev()`）。
fn is_dev() -> bool {
    cfg!(debug_assertions)
}

/// 依配置同步系统开机自启项（幂等；开发模式下除日志外什么都不做）。
pub fn sync(app: &AppHandle, config: &AppConfig) {
    if is_dev() {
        log::debug!(
            "开发模式：跳过开机自启同步（期望 enabled={}）",
            config.auto_start
        );
        return;
    }

    let manager = app.autolaunch();

    // 已经是目标状态就直接返回：`disable` 对着本来就不存在的启动项是否报错取决于
    // 平台实现，先查一次可以避免每次启动都留一条「删除失败」的噪声日志。
    // 查不通（`Err`）时照旧往下走，当作自愈路径。
    if let Ok(current) = manager.is_enabled() {
        if current == config.auto_start {
            return;
        }
    }

    let result = if config.auto_start {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(()) => log::info!("开机自启已同步 enabled={}", config.auto_start),
        // 失败不中断启动：设置页下一次改动会再同步一次。
        Err(err) => log::warn!("同步开机自启失败 enabled={}：{err}", config.auto_start),
    }
}
