use crate::services::persistence::{AppSettings, AppSettingsPatch};
use crate::AppState;
use tauri::{
    menu::{CheckMenuItem, ContextMenu, Menu, MenuItem},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Window,
};

pub struct TraySettingsItem(pub CheckMenuItem<tauri::Wry>);

#[derive(Default)]
pub struct SettingsWindowState(pub tokio::sync::Mutex<()>);

#[tauri::command]
pub async fn show_app_context_menu(window: Window, locale: String) -> Result<(), String> {
    let app = window.app_handle();
    let (refresh_label, settings_label, language) = if locale == "zh" {
        ("刷新", "设置", "zh")
    } else {
        ("Refresh", "Settings", "en")
    };
    let refresh = MenuItem::with_id(
        app,
        format!("app_refresh:{}", window.label()),
        refresh_label,
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let settings = MenuItem::with_id(
        app,
        format!("app_settings:{language}"),
        settings_label,
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    Menu::with_items(app, &[&refresh, &settings])
        .and_then(|menu| menu.popup(window))
        .map_err(|error| error.to_string())
}

// Window creation runs off the main thread as required by WebView2.
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
        "设置 · Drip Player"
    } else {
        "Settings · Drip Player"
    };
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(620.0, 460.0)
        .min_inner_size(520.0, 380.0)
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
    let player = state.0.lock().map_err(|error| error.to_string())?;
    Ok(player.settings.clone())
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
    let mut player = state.0.lock().map_err(|error| error.to_string())?;
    let previous = player.settings.minimize_to_tray;
    let mut settings = player.settings.clone();
    if let Some(theme) = patch.theme {
        if !matches!(theme.as_str(), "auto" | "light" | "dark") {
            return Err("Invalid theme".to_string());
        }
        settings.theme = theme;
    }
    if let Some(language) = patch.language {
        if !matches!(language.as_str(), "zh" | "en") {
            return Err("Invalid interface language".to_string());
        }
        settings.language = language;
    }
    if let Some(play_mode) = patch.play_mode {
        if !matches!(
            play_mode.as_str(),
            "sequential" | "random" | "repeat_one" | "repeat_all"
        ) {
            return Err("Invalid play mode".to_string());
        }
        settings.play_mode = play_mode;
    }
    if let Some(enabled) = patch.minimize_to_tray {
        settings.minimize_to_tray = enabled;
    }
    settings.revision = settings
        .revision
        .checked_add(1)
        .ok_or_else(|| "Settings revision overflow".to_string())?;
    let tray = app.state::<TraySettingsItem>();
    tray.0
        .set_checked(settings.minimize_to_tray)
        .map_err(|error| error.to_string())?;
    if let Err(error) = player.persistence.save_settings(&settings) {
        tray.0.set_checked(previous).map_err(|rollback_error| {
            format!("{error}; failed to restore tray state: {rollback_error}")
        })?;
        return Err(error);
    }
    player.settings = settings.clone();
    drop(player);
    app.emit("settings-changed", &settings)
        .map_err(|error| format!("Settings saved, but failed to notify windows: {error}"))?;
    Ok(settings)
}

#[tauri::command]
pub fn update_app_settings(app: AppHandle, patch: AppSettingsPatch) -> Result<AppSettings, String> {
    apply_settings(&app, patch)
}
