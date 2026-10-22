//! 本地项目的快捷启动：打开项目目录，或在新 cmd 窗口里执行配置好的启动命令。
//!
//! 这里**不接管进程**：命令跑在一个独立控制台窗口里，关掉那个窗口即结束。
//! 应用不持有句柄、不读输出 —— 用户要的就是「像自己开一个 cmd 再敲命令」。
//! 项目列表本身存在 `AppConfig.projects`（见 `config.rs`），由前端整份读写。

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

/// 新进程的控制台标志（Windows `CREATE_NEW_CONSOLE`）。
/// 发布版是 GUI 子系统进程，不显式建控制台的话 `cmd` 会没有可见窗口。
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// 校验项目目录：非空且确实是已存在的目录。
fn validate_path(path: &str) -> AppResult<&Path> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::Other("项目目录为空".into()));
    }
    let target = Path::new(trimmed);
    if !target.is_dir() {
        return Err(AppError::Other(format!("项目目录不存在：{trimmed}")));
    }
    Ok(target)
}

/// 在资源管理器里打开项目目录。
pub fn open_folder(app: &AppHandle, path: &str) -> AppResult<()> {
    let target = validate_path(path)?;
    app.opener()
        .open_path(target.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|err| AppError::Other(format!("打开目录失败：{err}")))
}

/// 在**新的 cmd 窗口**里执行一条启动命令，工作目录为项目目录。
///
/// 用 `cmd /k` 而不是 `/c`：命令结束后窗口留着，用户能看输出、也能继续输入，
/// 与「自己开 cmd 再敲命令」完全等价。
pub fn run_command(path: &str, command: &str) -> AppResult<()> {
    let target = validate_path(path)?;
    let command = command.trim();
    if command.is_empty() {
        return Err(AppError::Other("启动命令为空".into()));
    }

    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(["/k", command])
        .current_dir(target)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map_err(|err| AppError::Other(format!("启动命令失败：{err}")))?;

    log::info!("启动项目命令 cwd={} command={command}", target.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_missing_paths_are_rejected() {
        assert!(validate_path("").is_err());
        assert!(validate_path("   ").is_err());
        assert!(validate_path(r"D:\prism3-definitely-missing-dir").is_err());
    }

    #[test]
    fn temp_dir_is_a_valid_target() {
        let dir = std::env::temp_dir();
        assert!(validate_path(&dir.to_string_lossy()).is_ok());
    }

    #[test]
    fn empty_command_is_rejected_before_spawning() {
        // 空命令应在真正创建进程之前就失败，避免弹出一个什么都不做的 cmd 窗口。
        let dir = std::env::temp_dir();
        assert!(run_command(&dir.to_string_lossy(), "   ").is_err());
    }
}
