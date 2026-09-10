use crate::app_state::{lock, AppState};
use crate::application;
use crate::models::playback::{BrowserPlaybackReport, PlaybackPlan, PlaybackSnapshot};
use crate::services::audio_wrapper::AudioControl;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn get_state(state: State<AppState>) -> Result<PlaybackSnapshot, String> {
    Ok(lock(&state.playback)?.snapshot())
}

#[tauri::command]
pub async fn play_item(
    item_id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    application::play_entry(state.inner().clone(), app_handle, item_id).await
}

#[tauri::command]
pub async fn play_track_directly(
    path: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    application::play_path(state.inner().clone(), app_handle, path.into()).await
}

#[tauri::command]
pub async fn advance_playback(
    session_id: u64,
    backwards: bool,
    automatic: bool,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    application::advance(
        state.inner().clone(),
        app_handle,
        session_id,
        backwards,
        automatic,
    )
    .await
}

#[tauri::command]
pub fn report_browser_playback(
    report: BrowserPlaybackReport,
    state: State<AppState>,
) -> Result<PlaybackSnapshot, String> {
    let mut playback = lock(&state.playback)?;
    playback.report_browser(report)?;
    Ok(playback.snapshot())
}

#[tauri::command]
pub fn attach_browser_player(
    session_id: u64,
    owner: String,
    state: State<AppState>,
) -> Result<PlaybackSnapshot, String> {
    lock(&state.playback)?.attach_browser(session_id, owner)
}

#[tauri::command]
pub fn report_browser_error(
    session_id: u64,
    owner: String,
    message: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut playback = lock(&state.playback)?;
    if playback.session.as_ref().is_some_and(|session| {
        session.id == session_id
            && session.browser_owner.as_deref() == Some(owner.as_str())
            && matches!(session.plan, Some(PlaybackPlan::BrowserVideo { .. }))
    }) {
        playback.fail(session_id, message);
    }
    drop(playback);
    app_handle
        .emit("player-state-changed", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn pause(session_id: u64, state: State<'_, AppState>) -> Result<(), String> {
    audio_control(state.inner().clone(), session_id, AudioControl::Pause).await
}
#[tauri::command]
pub async fn resume(session_id: u64, state: State<'_, AppState>) -> Result<(), String> {
    audio_control(state.inner().clone(), session_id, AudioControl::Resume).await
}
#[tauri::command]
pub async fn seek(
    session_id: u64,
    position: f64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !position.is_finite() || !(0.0..=86400000.0).contains(&position) {
        return Err("Invalid seek position".into());
    }
    {
        let playback = lock(&state.playback)?;
        let session = playback
            .session
            .as_ref()
            .filter(|session| session.id == session_id)
            .ok_or("Playback session changed")?;
        if session.duration > 0.0 && position > session.duration {
            return Err("Seek position exceeds media duration".into());
        }
    }
    audio_control(
        state.inner().clone(),
        session_id,
        AudioControl::Seek(position),
    )
    .await
}
#[tauri::command]
pub async fn set_volume(
    session_id: u64,
    volume: f32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
        return Err("Invalid volume".into());
    }
    audio_control(
        state.inner().clone(),
        session_id,
        AudioControl::Volume(volume),
    )
    .await
}

async fn audio_control(
    state: AppState,
    session_id: u64,
    control: AudioControl,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let audio = lock(&state.playback)?.audio_for_session(session_id)?;
        audio.control(session_id, control)
    })
    .await
    .map_err(|error| error.to_string())?
}
