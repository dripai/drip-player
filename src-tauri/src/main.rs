#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app_state;
mod application;
mod handlers;
mod models;
mod services;

use app_state::AppState;
use handlers::{downloads::*, learning::*, library::*, platform::*, playback::*, settings::*};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

fn main() {
    let state =
        AppState::new().unwrap_or_else(|error| panic!("Failed to initialize application: {error}"));

    tauri::async_runtime::spawn_blocking(|| {
        services::media_remux::cleanup_remux_cache();
    });

    // 启动代理服务器
    tauri::async_runtime::spawn(async {
        services::stream_server::start_server(10001).await;
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .manage(SettingsWindowState::default())
        .manage(DownloadsWindowState::default())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let settings = state.settings.lock().unwrap();
                if settings.minimize_to_tray {
                    api.prevent_close();
                    window.hide().unwrap();
                } else {
                    for label in ["settings", "downloads"] {
                        if let Some(child) = window.get_webview_window(label) {
                            if let Err(error) = child.destroy() {
                                api.prevent_close();
                                let _ = window.emit("app-error", error.to_string());
                            }
                        }
                    }
                }
            }
        })
        .setup(|app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                services::toolchain::set_resource_dir(resource_dir);
            }

            let state = app.state::<AppState>().inner().clone();
            let state_for_menu = state.clone();

            // 系统托盘配置
            let initial_minimize_to_tray = state.settings.lock().unwrap().minimize_to_tray;
            let quit_i = MenuItem::with_id(app, "tray_quit", "退出", true, None::<&str>)?;
            let restore_i = MenuItem::with_id(app, "tray_restore", "恢复窗口", true, None::<&str>)?;
            let minimize_on_close_i = CheckMenuItem::with_id(
                app,
                "tray_minimize_on_close",
                "关闭时最小化",
                true,
                initial_minimize_to_tray,
                None::<&str>,
            )?;

            let tray_menu = Menu::with_items(app, &[&restore_i, &minimize_on_close_i, &quit_i])?;
            app.manage(TraySettingsItem(minimize_on_close_i));

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 菜单事件处理
            app.on_menu_event(move |app, event| {
                let event_id = event.id().as_ref();

                if event_id == "tray_quit" {
                    app.exit(0);
                } else if event_id == "tray_restore" {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                } else if event_id == "tray_minimize_on_close" {
                    let enabled = !state_for_menu.settings.lock().unwrap().minimize_to_tray;
                    if let Err(error) = apply_close_behavior(app, enabled) {
                        let _ = app.emit("app-error", error);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_playlist,
            refresh_playlist,
            get_playlist_file,
            rename_playlist_file,
            delete_playlist_file,
            play_item,
            play_track_directly,
            pause,
            resume,
            open_downloads_window,
            get_downloads,
            clear_download_history,
            submit_download,
            select_download_format,
            reparse_download,
            retry_download,
            cancel_download,
            add_local_files,
            pick_and_add_local_files,
            pick_and_add_folder,
            get_folder_tree,
            open_settings_window,
            get_app_settings,
            update_app_settings,
            set_download_directory,
            remove_track,
            clear_playlist,
            check_dependencies,
            seek,
            set_volume,
            get_state,
            check_mpv_available,
            open_platform_login,
            check_login_required,
            add_url_with_oauth,
            get_media_subtitles,
            advance_playback,
            report_browser_playback,
            attach_browser_player,
            report_browser_error,
            get_learning_settings,
            update_learning_settings,
            learning_secret_status,
            save_learning_secret,
            list_learning_transcripts,
            import_learning_subtitle,
            set_learning_favorite,
            list_learning_recordings,
            save_learning_recording,
            delete_learning_recording,
            explain_learning_cue,
            evaluate_learning_recording,
            start_learning_transcription,
            pending_learning_transcriptions,
            poll_learning_transcription
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<AppState>();
                let stop_downloads = (|| {
                    let _operation = app_state::lock(&state.directory_operation)?;
                    services::downloads::interrupt_all(&state, "app_closed")
                })();
                if let Err(error) = stop_downloads {
                    eprintln!("Failed to stop downloads on exit: {error}");
                }
                if let Err(error) =
                    app_state::lock(&state.playback).and_then(|mut playback| playback.stop())
                {
                    eprintln!("Failed to stop playback on exit: {error}");
                }
            }
        });
}
