use tauri::{AppHandle, State};

use crate::config::{AppConfig, AppState};
use crate::error::AppResult;

/// 读取当前配置。
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// 保存配置，同时更新内存状态与磁盘文件。
#[tauri::command]
pub fn set_config(app: AppHandle, state: State<'_, AppState>, config: AppConfig) -> AppResult<()> {
    *state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = config.clone();

    crate::config::save(&app, &config)
}

/// 恢复默认配置并落盘，返回恢复后的配置。
#[tauri::command]
pub fn reset_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppConfig> {
    let default = AppConfig::default();

    *state
        .config
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = default.clone();

    crate::config::save(&app, &default)?;
    Ok(default)
}
