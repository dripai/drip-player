use crate::app_state::AppState;
use crate::models::download::{DownloadAction, DownloadAuth, DownloadSnapshot};
use crate::services::downloads;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Default)]
pub struct DownloadsWindowState(pub tokio::sync::Mutex<()>);

#[tauri::command]
pub async fn open_downloads_window(app: AppHandle, locale: String) -> Result<(), String> {
    let state = app.state::<DownloadsWindowState>();
    let _opening = state.0.lock().await;
    if let Some(window) = app.get_webview_window("downloads") {
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    let main = app
        .get_webview_window("main")
        .ok_or("Main window is unavailable")?;
    WebviewWindowBuilder::new(&app, "downloads", WebviewUrl::App("index.html".into()))
        .title(if locale == "zh" {
            "下载 · 影子播放器"
        } else {
            "Downloads · Shadow Player"
        })
        .inner_size(740.0, 520.0)
        .min_inner_size(560.0, 360.0)
        .center()
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .parent(&main)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_downloads(state: State<'_, AppState>) -> Result<DownloadSnapshot, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || downloads::snapshot(&state))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn clear_download_history(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<usize, String> {
    let state = state.inner().clone();
    let removed = tauri::async_runtime::spawn_blocking(move || downloads::clear_history(&state))
        .await
        .map_err(|error| error.to_string())??;
    downloads::notify(&app);
    Ok(removed)
}

#[tauri::command]
pub async fn submit_download(
    state: State<'_, AppState>,
    app: AppHandle,
    url: String,
    auth: DownloadAuth,
) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || downloads::submit(&state, &app, url, auth))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn retry_download(
    state: State<'_, AppState>,
    app: AppHandle,
    job_id: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        downloads::start(&state, &app, &job_id, DownloadAction::Retry)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn select_download_format(
    state: State<'_, AppState>,
    app: AppHandle,
    job_id: String,
    option_id: String,
    attempt: u64,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        downloads::start(
            &state,
            &app,
            &job_id,
            DownloadAction::Select { option_id, attempt },
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn reparse_download(
    state: State<'_, AppState>,
    app: AppHandle,
    job_id: String,
    auth: DownloadAuth,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        downloads::start(&state, &app, &job_id, DownloadAction::Resolve(auth))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn cancel_download(
    state: State<'_, AppState>,
    app: AppHandle,
    job_id: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || downloads::cancel(&state, &job_id))
        .await
        .map_err(|error| error.to_string())?;
    downloads::notify(&app);
    result
}
