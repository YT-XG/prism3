mod ble;
mod commands;
mod config;
mod error;
mod file_index;
mod focus;
mod monitor;
mod status;
mod tray;

use std::sync::RwLock;

use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // 单实例插件必须最先注册；重复启动时唤起已有窗口。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(webview) = app.get_webview_window("main") {
                let _ = webview.show();
                let _ = webview.unminimize();
                let _ = webview.set_focus();
            }
        }));

    builder
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
        // 排除快捷小窗（灰条 + 面板）：其尺寸/位置由灰条窗口自行管理，且不应被恢复为可见。
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&["quick-panel", "quick-panel-body"])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();

            let initial_config = config::load(&handle);
            let quick_panel_enabled = initial_config.quick_panel;
            let file_search_enabled = initial_config.file_search;
            let file_search_roots = initial_config.file_search_roots.clone();
            app.manage(config::AppState {
                config: RwLock::new(initial_config),
            });

            // 系统监控状态：前端在监控页按需轮询时才会真正采集。
            app.manage(monitor::MonitorState::new());

            // 应用运行状态：由灰条窗口按固定间隔采样系统负载驱动（见 status.rs）。
            app.manage(status::AppStatusState::new());

            // 文件索引：禁用时不建索引、不占内存（见 file_index.rs）。
            app.manage(file_index::FileIndexState::new());

            // 蓝牙耳机：默认不扫描、不占链路，用户点「连接」才起会话线程（见 ble.rs）。
            app.manage(ble::BleState::new());

            // 蓝牙配置同步：把三个 BLE 键灌进状态镜像，并在「自动连接 + 已知目标」时起看门狗。
            // 这是**唯一**会在启动阶段碰蓝牙的路径：默认配置（不开自动连接）下零 BLE 开销，
            // 也正因为有它，「取出耳机 → 自动连上次那台」在重启之后依然成立。
            ble::sync_from_config(&handle);

            tray::init(app)?;

            // 窗口以 visible: false 启动，避免白屏闪烁。
            if let Some(webview) = app.get_webview_window("main") {
                webview.show()?;
            }

            // 快捷小窗窗口按需创建：仅在配置启用时创建，功能关闭则不占用 WebView。
            if quick_panel_enabled {
                if let Err(err) = commands::ensure_quick_panel_windows(&handle) {
                    log::error!("创建快捷小窗窗口失败：{err}");
                }
            }

            // 文件搜索：先把上次落盘的索引读进来（秒开），再在后台重扫一遍。
            if file_search_enabled {
                let state = app.state::<file_index::FileIndexState>();
                state.set_enabled(true);
                std::thread::spawn(move || {
                    let state = handle.state::<file_index::FileIndexState>();
                    file_index::load_from_disk(&handle, &state);
                    let (roots, missing_roots) =
                        file_index::resolve_roots(&handle, &file_search_roots);
                    if let Err(err) =
                        file_index::rebuild_blocking(&handle, &state, roots, missing_roots)
                    {
                        log::error!("构建文件索引失败：{err}");
                        state.fail(format!("构建文件索引失败：{err}"));
                        state.broadcast(&handle);
                    }
                });
            }

            log::info!("Prism3 启动完成");
            Ok(())
        })
        .on_window_event(|window, event| {
            // 面板失去焦点即退出「输入聚焦」模式：不管是被外部夺焦（Alt+Tab）还是
            // 窗口被隐藏，都必须让窗口回到不可聚焦，否则会残留一个会抢焦点的常驻窗口。
            if let WindowEvent::Focused(false) = event {
                if window.label() == "quick-panel-body" {
                    let app = window.app_handle();
                    if let Err(err) = focus::release(app) {
                        log::warn!("收回面板聚焦模式失败：{err}");
                    }
                    let _ = app.emit_to("quick-panel-body", "quick-panel:focus-mode", false);
                }
            }

            if let WindowEvent::CloseRequested { api, .. } = event {
                // 仅主窗口走「关闭到托盘」逻辑；快捷小窗窗口由命令主动销毁。
                if window.label() != "main" {
                    return;
                }
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
            commands::reset_config,
            commands::set_quick_panel_dock,
            commands::set_quick_panel_geometry,
            commands::set_quick_panel_body_position,
            commands::open_quick_panel_body,
            commands::close_quick_panel_body,
            commands::hide_quick_panel_body,
            commands::get_system_snapshot,
            commands::get_system_statics,
            commands::get_app_status,
            commands::refresh_app_status,
            commands::search_files,
            commands::get_file_index_state,
            commands::rebuild_file_index,
            commands::cancel_file_index,
            commands::open_search_result,
            commands::focus_quick_panel_body,
            commands::blur_quick_panel_body,
            commands::get_ble_state,
            commands::connect_earbuds,
            commands::disconnect_earbuds,
            commands::scan_ble_devices,
            commands::set_ble_auto_connect,
            commands::set_ble_active_device,
            commands::add_ble_device,
            commands::remove_ble_device,
            commands::refresh_earbud_state,
            commands::read_earbud_battery,
            commands::set_noise_mode,
            commands::set_anc_gradient,
            commands::set_adaptive_anc,
            commands::set_eq_preset,
            commands::set_game_mode,
            commands::set_dual_device
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {});
}
