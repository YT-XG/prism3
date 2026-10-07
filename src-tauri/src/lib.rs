mod commands;
mod config;
mod error;
mod tray;

use std::sync::RwLock;

use tauri::{Manager, WindowEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例插件必须最先注册；重复启动时唤起已有窗口。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(webview) = app.get_webview_window("main") {
                let _ = webview.show();
                let _ = webview.unminimize();
                let _ = webview.set_focus();
            }
        }))
        // 唯一的日志出口：Rust 与前端（经 @tauri-apps/plugin-log）写入同一个文件。
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("prism3_lib", log::LevelFilter::Debug)
                // 日志时间戳用本地时区，默认是 UTC。
                .timezone_strategy(TimezoneStrategy::UseLocal)
                // 默认只保留单个 40KB 文件，改成保留全部历史并按日期命名。
                .rotation_strategy(RotationStrategy::KeepAll)
                .max_file_size(5 * 1024 * 1024)
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("prism3".into()),
                    }),
                    // 供前端 attachConsole 订阅。
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();

            app.manage(config::AppState {
                config: RwLock::new(config::load(&handle)),
            });

            tray::init(app)?;

            // 窗口以 visible: false 启动，避免白屏闪烁。
            if let Some(webview) = app.get_webview_window("main") {
                webview.show()?;
            }

            log::info!("Prism3 启动完成");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let close_to_tray = app
                    .state::<config::AppState>()
                    .config
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .close_to_tray;

                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_config,
            commands::reset_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
