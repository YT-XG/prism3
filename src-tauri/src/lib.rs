mod autostart;
mod ble;
mod clipboard;
mod commands;
mod config;
mod db;
mod error;
mod file_index;
mod focus;
mod models;
mod monitor;
mod note_image;
mod note_io;
mod paste;
mod repo;
mod status;
mod tray;
mod updater;

use std::sync::RwLock;

use tauri::{AppHandle, Emitter, Manager, WindowEvent};
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
        // 可见性也不交给插件恢复：默认旗标会在启动时按上次退出时的 `visible` 调
        // `show() + set_focus()`，那会在 `setup` 决定「开机自启不弹窗」之前就把主窗口弹出来。
        // 主窗口什么时候显示只由本文件（`setup`）与 `tray.rs`（点托盘）决定。
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&["quick-panel", "quick-panel-body"])
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // 开机自启（Windows 即 HKCU 的 Run 项）；真正的注册由 `autostart::sync` 按配置执行。
        // 启动项带 `--autostart` 参数：`setup` 据此判断本次是开机拉起，只驻留托盘不弹主窗口。
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![autostart::LAUNCH_FLAG]),
        ))
        // 速记图片协议：笔记正文内嵌 `http://prism3-note.localhost/<hash>.<ext>`，
        // 按「配置目录/notes/images/<文件名>」读取（URL 不含绝对路径，迁数据目录后仍有效）；
        // 文件名严格校验为「16 位十六进制哈希 + 白名单扩展」，杜绝路径穿越。
        // **不启用 `protocol-asset`、不动 CSP**（当前 `csp: null`）；将来若启用 CSP，
        // 必须把 `http://prism3-note.localhost` 加进 `img-src`。
        .register_uri_scheme_protocol("prism3-note", |ctx, request| {
            let name = request.uri().path().trim_start_matches('/').to_string();
            if !note_image::valid_note_image_name(&name) {
                return tauri::http::Response::builder()
                    .status(400)
                    .body(Vec::new())
                    .unwrap();
            }
            let mime = match name.rsplit_once('.').map(|(_, ext)| ext.to_lowercase()) {
                Some(ext) if ext == "png" => "image/png",
                Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
                Some(ext) if ext == "webp" => "image/webp",
                Some(ext) if ext == "bmp" => "image/bmp",
                Some(ext) if ext == "gif" => "image/gif",
                _ => "application/octet-stream",
            };
            let path = ctx
                .app_handle()
                .path()
                .app_config_dir()
                .map(|dir| dir.join("notes").join("images").join(&name));
            match path.ok().and_then(|path| std::fs::read(path).ok()) {
                Some(bytes) => tauri::http::Response::builder()
                    .header("Content-Type", mime)
                    .header("Access-Control-Allow-Origin", "*")
                    // 内容哈希命名 ⇒ 同名同内容，可长缓存。
                    .header("Cache-Control", "public, max-age=31536000, immutable")
                    .body(bytes)
                    .unwrap(),
                None => tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap(),
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();

            // 本次进程是不是开机自启项拉起的（命令行里的 `--autostart`，见 autostart.rs）。
            // 早算一次，后面决定主窗口要不要显示。
            let launched_by_autostart = autostart::launched_by_autostart();

            let initial_config = config::load(&handle);
            let quick_panel_enabled = initial_config.quick_panel;
            let file_search_enabled = initial_config.file_search;
            let file_search_roots = initial_config.file_search_roots.clone();
            app.manage(config::AppState {
                config: RwLock::new(initial_config.clone()),
            });

            // 统一存储：速记（笔记）与剪贴板历史共用一个 SQLite 库（见 db.rs）。
            // 库与 config.json / 剪贴板图片目录同放在 app_config_dir 下。
            let db_path = handle.path().app_config_dir()?.join("prism3.db");
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match db::init(&db_path) {
                Ok(conn) => {
                    app.manage(db::DbState::new(conn));
                }
                Err(err) => {
                    // 库打不开是致命问题：记日志并让启动失败（不回退到内存库，避免数据静默丢失）。
                    log::error!("初始化数据库失败 path={} err={err}", db_path.display());
                    return Err(err.into());
                }
            }

            // 系统监控状态：前端在监控页按需轮询时才会真正采集。
            app.manage(monitor::MonitorState::new());

            // 应用运行状态：由灰条窗口按固定间隔采样系统负载驱动（见 status.rs）。
            app.manage(status::AppStatusState::new());

            // 文件索引：禁用时不建索引、不占内存（见 file_index.rs）。
            app.manage(file_index::FileIndexState::new());

            // 蓝牙耳机：默认不扫描、不占链路，用户点「连接」才起会话线程（见 ble.rs）。
            app.manage(ble::BleState::new());

            // 剪贴板历史：与「启用与否」无关地先读一次落盘历史（几毫秒），
            // 这样「先禁用、之后重新启用」也能立刻看到旧记录。
            app.manage(clipboard::ClipboardState::new());
            {
                let state = app.state::<clipboard::ClipboardState>();
                clipboard::load_from_db(&handle, &state);
            }

            // 蓝牙配置同步：把三个 BLE 键灌进状态镜像，并在「自动连接 + 已知目标」时起看门狗。
            // 这是**唯一**会在启动阶段碰蓝牙的路径：默认配置（不开自动连接）下零 BLE 开销，
            // 也正因为有它，「取出耳机 → 自动连上次那台」在重启之后依然成立。
            ble::sync_from_config(&handle);

            // 剪贴板：启用则起采集线程（默认关闭时零开销）。
            clipboard::sync(&handle, &initial_config);

            // 开机自启：默认配置即开，这里把系统启动项对齐到配置（开发模式跳过）。
            autostart::sync(&handle, &initial_config);

            // 自动更新：托管状态 + 发布版启动 3s 后静默检查一次。
            // 开发模式默认不检查、也不执行安装；联调时设 PRISM3_UPDATE_ANCHOR（见 updater.rs）。
            updater::init(&handle);
            updater::schedule_startup_check(&handle);

            tray::init(app)?;

            // 速记启动维护（启动 15s 后跑，避开启动期与其它 DB 操作撞车）：
            // ① 按保留天数清理回收站；② 全库双链索引重建（幂等）。都不阻塞启动。
            {
                let maintenance_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    let days = maintenance_handle
                        .state::<config::AppState>()
                        .config
                        .read()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .note_trash_retention_days;
                    let state = maintenance_handle.state::<db::DbState>();
                    let conn = state.lock();
                    if let Ok(purged) = repo::note::purge_expired(&conn, days as u64) {
                        if purged > 0 {
                            log::info!("启动维护：清理回收站过期速记 {purged} 条");
                        }
                    }
                    if let Err(err) = repo::note_link::reindex_all(&conn) {
                        log::warn!("启动维护：重建速记双链索引失败：{err}");
                    }
                });
            }

            // 窗口以 visible: false 启动，避免白屏闪烁。
            // 显示前先把底色设成当前主题：窗口比页面首帧早得多，默认白色在暗色主题下会闪。
            if let Some(webview) = app.get_webview_window("main") {
                sync_window_background(&handle, initial_config.theme);
                // 开机自启拉起的进程只驻留托盘：用户是按了开机，不是要打开应用。
                // 需要主窗口时点托盘图标（`tray.rs`），或再启动一次（single-instance 会唤起）。
                if launched_by_autostart {
                    log::info!("由开机自启拉起：主窗口保持隐藏，仅驻留托盘");
                } else {
                    webview.show()?;
                }
            }

            // 快捷小窗窗口按需创建：仅在配置启用时创建，功能关闭则不占用 WebView。
            if quick_panel_enabled {
                if let Err(err) = commands::ensure_quick_panel_windows(&handle) {
                    log::error!("创建快捷小窗窗口失败：{err}");
                }
            }

            // 文件搜索：落盘索引新鲜就直接复用（不重扫），过期或索引根变了才在后台重建。
            if file_search_enabled {
                let state = app.state::<file_index::FileIndexState>();
                state.set_enabled(true);
                std::thread::spawn(move || {
                    let state = handle.state::<file_index::FileIndexState>();
                    let (roots, missing_roots) =
                        file_index::resolve_roots(&handle, &file_search_roots);
                    let reused = file_index::disk_index_is_reusable(&handle, &roots)
                        && file_index::load_from_disk(&handle, &state);
                    if reused {
                        log::info!(
                            "落盘文件索引在有效期内（{} 小时），跳过重扫",
                            file_index::INDEX_TTL_SECS / 3600
                        );
                        return;
                    }
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
            commands::read_wallpaper,
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
            commands::set_dual_device,
            commands::get_clipboard_state,
            commands::search_clipboard,
            commands::get_clipboard_text,
            commands::get_clipboard_image,
            commands::copy_clipboard_entry,
            commands::paste_clipboard_entry,
            commands::update_clipboard_text,
            commands::delete_clipboard_entry,
            commands::set_clipboard_pinned,
            commands::clear_clipboard_history,
            commands::prune_clipboard_history,
            commands::list_notes,
            commands::get_note,
            commands::create_note_in,
            commands::update_note,
            commands::set_note_icon,
            commands::set_note_folder,
            commands::trash_note,
            commands::restore_note,
            commands::purge_note,
            commands::purge_all_trashed_notes,
            commands::purge_expired_notes,
            commands::search_notes,
            commands::get_note_links,
            commands::rebuild_note_links,
            commands::list_note_folders,
            commands::create_note_folder,
            commands::rename_note_folder,
            commands::delete_note_folder,
            commands::reorder_note_folders,
            commands::list_tags,
            commands::create_tag,
            commands::rename_tag,
            commands::delete_tag,
            commands::get_note_tags,
            commands::set_note_tags,
            commands::list_note_tags,
            commands::import_note_image,
            commands::gc_orphan_note_images,
            commands::export_notes,
            commands::import_notes,
            commands::import_notes_cancel,
            commands::get_update_state,
            commands::check_update,
            commands::reinstall_latest,
            commands::pause_update,
            commands::resume_update,
            commands::cancel_update,
            commands::install_update,
            commands::open_update_installer,
            commands::open_update_download_url
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {});
}

/// 把主窗口底色同步成主题的背景色。
///
/// 窗口比页面首帧早得多（前端在 dev 下要几百毫秒到几秒），默认的纯白底色在暗色主题下
/// 会明显闪一下。这里按主题设置窗口与 WebView 的底色：亮色 `#f7f8fa`、暗色 `#12131b`
/// （与 `index.css` 里 `--bg-page` 的默认取值一致）。
pub(crate) fn sync_window_background(app: &AppHandle, theme: config::Theme) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let dark = match theme {
        config::Theme::Dark => true,
        config::Theme::Light => false,
        // 「跟随系统」时窗口主题就是系统主题（WebView2 跟着系统走）。
        config::Theme::System => matches!(window.theme(), Ok(tauri::Theme::Dark)),
    };
    let color = if dark {
        tauri::window::Color(18, 19, 27, 255)
    } else {
        tauri::window::Color(247, 248, 250, 255)
    };
    if let Err(err) = window.set_background_color(Some(color)) {
        log::warn!("同步窗口底色失败：{err}");
    }
}
