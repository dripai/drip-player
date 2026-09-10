use crate::app_state::{lock, AppState};
use crate::models::settings::{AppSettings, AppSettingsPatch};
use crate::services::directory_library::{self, DirectoryUpdate};
use tauri::{
    menu::CheckMenuItem, AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
};

pub struct TraySettingsItem(pub CheckMenuItem<tauri::Wry>);

#[derive(Default)]
pub struct SettingsWindowState(pub tokio::sync::Mutex<()>);

// Window creation runs off the main thread as required by WebView2.
#[tauri::command]
pub async fn open_settings_window(app: AppHandle, locale: String) -> Result<(), String> {
    let state = app.state::<SettingsWindowState>();
    let _opening = state.0.lock().await;
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }

    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable".to_string())?;
    let title = if locale == "zh" {
        "设置 · 影子播放器"
    } else {
        "Settings · Shadow Player"
    };
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(800.0, 620.0)
        .min_inner_size(660.0, 480.0)
        .center()
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .parent(&main)
        .map_err(|error| error.to_string())?
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_app_settings(state: State<AppState>) -> Result<AppSettings, String> {
    Ok(lock(&state.settings)?.clone())
}

pub fn apply_close_behavior(app: &AppHandle, enabled: bool) -> Result<AppSettings, String> {
    apply_settings(
        app,
        AppSettingsPatch {
            minimize_to_tray: Some(enabled),
            ..Default::default()
        },
    )
}

fn apply_settings(app: &AppHandle, patch: AppSettingsPatch) -> Result<AppSettings, String> {
    let state = app.state::<AppState>();
    let mut current = lock(&state.settings)?;
    let settings = current.patched(patch)?;
    lock(&state.database)?.save_settings(&settings)?;
    *current = settings.clone();
    drop(current);
    // Native menu methods dispatch to the main thread. Never wait for them while
    // holding settings/database locks; a window-close callback reads settings too.
    let sync_app = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            // Read the latest committed value when this callback runs, so queued
            // callbacks from older writes cannot restore an obsolete tray value.
            let state = sync_app.state::<AppState>();
            let enabled = lock(&state.settings)?.minimize_to_tray;
            sync_app
                .state::<TraySettingsItem>()
                .0
                .set_checked(enabled)
                .map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            let _ = sync_app.emit(
                "app-error",
                format!("Settings saved, but tray synchronization failed: {error}"),
            );
        }
    })
    .map_err(|error| {
        format!("Settings saved, but tray synchronization could not be scheduled: {error}")
    })?;
    app.emit("settings-changed", &settings)
        .map_err(|error| format!("Settings saved, but failed to notify windows: {error}"))?;
    Ok(settings)
}

#[tauri::command]
pub fn update_app_settings(app: AppHandle, patch: AppSettingsPatch) -> Result<AppSettings, String> {
    apply_settings(&app, patch)
}

pub fn notify_directory_update(
    app: &AppHandle,
    update: DirectoryUpdate,
) -> Result<AppSettings, String> {
    let mut errors: Vec<String> = update.playback_error.into_iter().collect();
    for result in [
        app.emit("settings-changed", &update.settings),
        app.emit("playlist-updated", ()),
        app.emit("downloads-updated", ()),
        app.emit("player-state-changed", ()),
    ] {
        if let Err(error) = result {
            errors.push(format!("目录和播放列表已更新，但窗口通知失败：{error}"));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok(update.settings)
}

#[tauri::command]
pub async fn set_download_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<AppSettings, String> {
    let state = state.inner().clone();
    let update = tauri::async_runtime::spawn_blocking(move || {
        directory_library::update_directory(&state, Some(path.into()))
    })
    .await
    .map_err(|error| error.to_string())?;
    crate::services::downloads::notify(&app);
    notify_directory_update(&app, update?)
}
