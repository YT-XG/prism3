use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::AppResult;

/// 主题偏好，序列化后与前端 `Theme` 联合类型一致：`"system" | "light" | "dark"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// 快捷小窗吸附的屏幕边缘。不含底部（底部与任务栏冲突且非习惯位置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DockEdge {
    #[default]
    Right,
    Left,
    Top,
}

/// 应用配置，持久化为 `%APPDATA%\<identifier>\config.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub theme: Theme,
    pub language: String,
    /// 关闭主窗口时隐藏到托盘而不是退出。
    pub close_to_tray: bool,
    pub last_opened: Option<String>,
    /// 是否启用屏幕边缘的快捷小窗。
    pub quick_panel: bool,
    /// 快捷小窗吸附的屏幕边缘。
    pub quick_panel_edge: DockEdge,
    /// 快捷小窗沿吸附边缘的偏移（逻辑像素）：`left`/`right` 时为纵向 y，`top` 时为横向 x。
    /// 由面板窗口独占写入。
    pub quick_panel_y: Option<i32>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            language: "zh-CN".to_string(),
            close_to_tray: true,
            last_opened: None,
            quick_panel: false,
            quick_panel_edge: DockEdge::default(),
            quick_panel_y: None,
        }
    }
}

/// 由 Tauri 托管的应用状态。
pub struct AppState {
    pub config: RwLock<AppConfig>,
}

fn config_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_config_dir()?.join("config.json"))
}

/// 读取配置。文件不存在或内容损坏时回退到默认值，不中断启动。
pub fn load(app: &AppHandle) -> AppConfig {
    let Ok(path) = config_path(app) else {
        return AppConfig::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// 写入配置。先写临时文件再重命名，避免写入中途损坏原文件。
pub fn save(app: &AppHandle, config: &AppConfig) -> AppResult<()> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(config)?)?;
    std::fs::rename(&tmp, &path)?;

    log::debug!(target: "prism3_lib::config", "配置已保存 path={}", path.display());
    Ok(())
}
