//! 开机自启：配置里的 `auto_start` 就是系统启动项的期望状态。
//!
//! 这里只有「配置 → 系统」一条同步路径（近似幂等）：启动时跑一次（默认配置即开），
//! 设置页改开关时再跑一次（见 `commands.rs` 的 `set_config` / `reset_config`）。
//! 落地点是 `tauri-plugin-autostart`（Windows 上即 `HKCU\...\CurrentVersion\Run`）。
//!
//! **自启项带 `--autostart` 参数**：`lib.rs::setup` 用它区分「开机拉起」与「用户手动双击」，
//! 前者只驻留托盘、不弹主窗口（见 [`LAUNCH_FLAG`]）。参数由插件的
//! `init(.., Some(vec![...]))` 写进系统启动项，所以启用时 `sync` **每次启动都重写一遍**——
//! `is_enabled()` 只回答「这项在不在」，不比对命令行，旧版本留下的裸路径启动项
//! 只能靠这次重写补上参数。
//!
//! **开发模式一律不注册**：`tauri dev` 下当前可执行文件在 `target\debug` 里，
//! 把它写进启动项的结果是「开机启动一个只有在开发环境里才跑得起来的调试程序」，
//! 用户还会以为功能坏了。判定用 `cfg!(debug_assertions)`（`pnpm tauri dev` 走 dev
//! profile 必然为 true，`pnpm tauri build` 走 release profile 为 false），而不是
//! `tauri::is_dev()`——后者看的是 tauri 的 `custom-protocol` 特性，本仓库并未声明它。

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::config::AppConfig;

/// 开机自启拉起进程时附加的命令行参数。
///
/// 它同时是「这次不要弹主窗口」的唯一信号：用户手动双击时命令行里没有它；
/// 系统启动项由 [`sync`] 写入，参数就写在那里。
pub const LAUNCH_FLAG: &str = "--autostart";

/// 是否是开发模式构建（见模块头：不用 `tauri::is_dev()`）。
fn is_dev() -> bool {
    cfg!(debug_assertions)
}

/// 本次进程是否由开机自启项拉起（只看命令行，见 [`LAUNCH_FLAG`]）。
pub fn launched_by_autostart() -> bool {
    has_launch_flag(std::env::args())
}

/// 只认完整参数：`--autostart-foo` 之类不能被误判成自启。
fn has_launch_flag(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == LAUNCH_FLAG)
}

/// 依配置同步系统开机自启项（开发模式下除日志外什么都不做）。
pub fn sync(app: &AppHandle, config: &AppConfig) {
    if is_dev() {
        log::debug!(
            "开发模式：跳过开机自启同步（期望 enabled={}）",
            config.auto_start
        );
        return;
    }

    let manager = app.autolaunch();

    if config.auto_start {
        // 一律重写一次：`is_enabled()` 并不比对命令行，所以「已注册但没带 `LAUNCH_FLAG`」
        // 的旧启动项不会被发现，只能靠重写升级；顺带也能把用户在系统设置里禁用的项恢复。
        // 写入同一个值本身是幂等的，代价只有一次注册表写。
        match manager.enable() {
            Ok(()) => log::debug!("开机自启已同步 enabled=true（参数 {LAUNCH_FLAG}）"),
            // 失败不中断启动：设置页下一次改动会再同步一次。
            Err(err) => log::warn!("同步开机自启失败 enabled=true：{err}"),
        }
        return;
    }

    // 关闭时先查一次：本来就没注册就不去删，省掉每次启动一条无意义的注册表操作。
    // 查不通（`Err`）时照旧往下走，当作自愈路径。
    if let Ok(current) = manager.is_enabled() {
        if !current {
            return;
        }
    }

    match manager.disable() {
        Ok(()) => log::info!("开机自启已同步 enabled=false"),
        Err(err) => log::warn!("同步开机自启失败 enabled=false：{err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{has_launch_flag, LAUNCH_FLAG};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn detects_flag_after_executable_path() {
        assert!(has_launch_flag(args(&[
            r"C:\Program Files\Prism3\prism3.exe",
            LAUNCH_FLAG,
        ])));
    }

    #[test]
    fn ignores_manual_launch() {
        assert!(!has_launch_flag(args(&[
            r"C:\Program Files\Prism3\prism3.exe"
        ])));
        assert!(!has_launch_flag(Vec::new()));
    }

    #[test]
    fn does_not_match_partial_tokens() {
        assert!(!has_launch_flag(args(&[
            "--autostart-foo",
            "--no-autostart",
            "autostart",
        ])));
    }
}
