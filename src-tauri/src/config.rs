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

/// 一台已知的蓝牙 LE 耳机。
///
/// `name` 是从设备读回来的**缓存**友好名（耳机不广播名字，真名只在连接后的 GATT
/// Device Name 特征里，见 `ble.rs`）；`last_connected_at` 是最近一次成功建立的秒级
/// 时间戳，用于「连接上次已连接的」；`adaptive_anc` 是**本地记忆**的自适应降噪开关
/// —— 设备从不上报这一位（见 `ble.rs::adaptive_anc_request`），所以它只能存在这里。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BleDeviceEntry {
    pub address: String,
    pub name: Option<String>,
    pub last_connected_at: Option<u64>,
    /// 自适应降噪开关的本地记忆；缺省 `false`，因此老配置文件照常可读
    /// （容器级 `serde(default)` 与 `AppConfig` 的写法一致）。
    pub adaptive_anc: bool,
}

/// 应用配置，持久化为 `%APPDATA%\<identifier>\config.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub theme: Theme,
    /// 配色预设 id（前端 `THEME_PRESETS` 的 id）；未知值由前端回退到默认。
    pub theme_preset: String,
    /// 强调色 `#rrggbb`；空串表示用默认强调色。
    pub accent_color: String,
    /// 壁纸图片的绝对路径；空串表示不用壁纸（只显示渐变底）。
    pub wallpaper: String,
    /// 壁纸模糊半径（px，0 = 不模糊）。
    pub wallpaper_blur: u32,
    /// 壁纸蒙版强度（百分比，0 = 不压暗，越大文字越清晰）。
    pub wallpaper_veil: u32,
    /// 卡片玻璃不透明度（百分比，越小越透出背景）。
    pub glass_opacity: u32,
    pub language: String,
    /// 关闭主窗口时隐藏到托盘而不是退出。
    pub close_to_tray: bool,
    /// 开机时自动启动（**默认开启**；开发模式不注册，见 `autostart.rs`）。
    pub auto_start: bool,
    pub last_opened: Option<String>,
    /// 是否启用屏幕边缘的快捷小窗。
    pub quick_panel: bool,
    /// 快捷小窗吸附的屏幕边缘。
    pub quick_panel_edge: DockEdge,
    /// 快捷小窗沿吸附边缘的偏移（逻辑像素）：`left`/`right` 时为纵向 y，`top` 时为横向 x。
    /// 由面板窗口独占写入。
    pub quick_panel_y: Option<i32>,
    /// 是否启用本地文件搜索（自建文件名索引，见 `file_index.rs`）。
    pub file_search: bool,
    /// 文件搜索的索引根目录；为空表示使用桌面 / 文档 / 下载 / 图片四个常用目录。
    pub file_search_roots: Vec<String>,
    /// 是否监视耳机并自动连接（取出耳机即连上「上次已连接的」，见 `ble.rs`）。
    pub ble_auto_connect: bool,
    /// 当前 / 上次已连接的耳机地址；为空表示还不知道要连谁（此时看门狗不起）。
    pub ble_active_address: Option<String>,
    /// 已知的耳机设备列表（缓存的名字 + 最近连接时间）。
    pub ble_devices: Vec<BleDeviceEntry>,
    /// 是否记录剪贴板历史（文本 + 图片，见 `clipboard.rs`）；**默认开启**。
    pub clipboard_history: bool,
    /// 剪贴板历史的保留天数；`0` 表示永久保留。
    pub clipboard_retention_days: u32,
    /// 是否把剪贴板里的图片也计入历史（截图等体积较大，可单独关掉）。
    pub clipboard_capture_images: bool,
    /// 点击一条记录时是否自动「回到上一个焦点并粘贴」；关闭后只复制到剪贴板。
    pub clipboard_paste_on_click: bool,
    /// 首页 Bento 布局（前端 `serializeHomeLayout` 生成的 JSON 字符串）。
    /// 空串表示用默认布局；解析失败也回退默认，因此后端只当不透明字符串存。
    pub home_layout: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            theme_preset: "plain".to_string(),
            accent_color: String::new(),
            wallpaper: String::new(),
            wallpaper_blur: 18,
            wallpaper_veil: 40,
            glass_opacity: 60,
            language: "zh-CN".to_string(),
            close_to_tray: true,
            auto_start: true,
            last_opened: None,
            quick_panel: false,
            quick_panel_edge: DockEdge::default(),
            quick_panel_y: None,
            file_search: false,
            file_search_roots: Vec::new(),
            ble_auto_connect: false,
            ble_active_address: None,
            ble_devices: Vec::new(),
            clipboard_history: true,
            clipboard_retention_days: 30,
            clipboard_capture_images: true,
            clipboard_paste_on_click: true,
            home_layout: String::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_keys_are_camel_case_in_the_config_file() {
        // 前端按这些键名读写配置（与 `AppConfig` 既有的 camelCase 约定一致），改名即破坏契约。
        let config = AppConfig {
            ble_auto_connect: true,
            ble_active_address: Some("12:02:03:04:7C:0F".into()),
            ble_devices: vec![BleDeviceEntry {
                address: "12:02:03:04:7C:0F".into(),
                name: Some("StarRing Ultra_BLE".into()),
                last_connected_at: Some(1_700_000_000),
                adaptive_anc: true,
            }],
            ..AppConfig::default()
        };
        let value = serde_json::to_value(&config).expect("配置应当可序列化");
        assert_eq!(value["bleAutoConnect"], true);
        assert_eq!(value["bleActiveAddress"], "12:02:03:04:7C:0F");

        let device = &value["bleDevices"][0];
        let mut keys: Vec<&str> = device
            .as_object()
            .expect("设备项应当是对象")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["adaptiveAnc", "address", "lastConnectedAt", "name"]
        );
        assert_eq!(device["lastConnectedAt"], 1_700_000_000u64);
        assert_eq!(device["adaptiveAnc"], true);
    }

    #[test]
    fn old_config_files_without_ble_keys_stay_valid() {
        // 老配置文件里没有这些键：必须回落到默认值，而不是整个配置反序列化失败
        // （`load()` 一旦失败就整份退回默认，用户的主题 / 快捷小窗设置会一起丢）。
        let config: AppConfig =
            serde_json::from_str(r#"{"theme":"dark","fileSearch":true}"#).expect("老配置应当可读");
        assert_eq!(config.theme, Theme::Dark);
        assert!(config.file_search);
        assert!(!config.ble_auto_connect, "缺省 = 不自动连接");
        assert_eq!(config.ble_active_address, None);
        assert!(config.ble_devices.is_empty());
        assert_eq!(
            config,
            AppConfig {
                theme: Theme::Dark,
                file_search: true,
                ..AppConfig::default()
            }
        );

        // 设备项自身缺字段时同样回落（`BleDeviceEntry` 也是 `serde(default)`）：
        // 尤其是后加的 `adaptiveAnc` —— 老配置里必然没有它，必须读成 `false`
        // 而不是让整份配置报废。
        let entry: BleDeviceEntry = serde_json::from_str(r#"{"address":"12:02:03:04:7C:0F"}"#)
            .expect("缺少可选项的设备项应当可读");
        assert_eq!(entry.name, None);
        assert_eq!(entry.last_connected_at, None);
        assert!(!entry.adaptive_anc, "缺省 = 自适应降噪关");

        // 老配置里已经有设备条目、但没有 `adaptiveAnc` 时同样成立。
        let config: AppConfig = serde_json::from_str(
            r#"{"bleDevices":[{"address":"12:02:03:04:7C:0F","name":"StarRing Ultra_BLE","lastConnectedAt":7}]}"#,
        )
        .expect("带老设备条目的配置应当可读");
        assert_eq!(config.ble_devices.len(), 1);
        assert_eq!(
            config.ble_devices[0].name.as_deref(),
            Some("StarRing Ultra_BLE")
        );
        assert_eq!(config.ble_devices[0].last_connected_at, Some(7));
        assert!(!config.ble_devices[0].adaptive_anc);
    }

    #[test]
    fn clipboard_keys_are_camel_case_and_default_to_sane_values() {
        // 前端按这些键名读写剪贴板设置（与 `AppConfig` 的 camelCase 约定一致），改名即破坏契约。
        let value = serde_json::to_value(AppConfig::default()).expect("配置应当可序列化");
        assert_eq!(value["clipboardHistory"], true);
        assert_eq!(value["clipboardRetentionDays"], 30);
        assert_eq!(value["clipboardCaptureImages"], true);
        assert_eq!(value["clipboardPasteOnClick"], true);

        // 老配置里没有这四个键：必须回落到默认值，而不是整份配置反序列化失败。
        let config: AppConfig =
            serde_json::from_str(r#"{"fileSearch":true}"#).expect("老配置应当可读");
        assert!(config.clipboard_history);
        assert_eq!(config.clipboard_retention_days, 30);
        assert!(config.clipboard_capture_images);
        assert!(config.clipboard_paste_on_click);
    }

    #[test]
    fn home_layout_key_is_camel_case_and_defaults_to_empty() {
        // 首页布局整份以 JSON 字符串存在这个键里（解析归前端），后端只负责原样存取。
        let value = serde_json::to_value(AppConfig::default()).expect("配置应当可序列化");
        assert_eq!(value["homeLayout"], "");

        // 老配置里没有这个键：必须回落到空串，而不是整份配置反序列化失败。
        let config: AppConfig =
            serde_json::from_str(r#"{"fileSearch":true}"#).expect("老配置应当可读");
        assert!(config.home_layout.is_empty());
    }
}
