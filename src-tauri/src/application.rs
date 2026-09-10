use crate::app_state::{lock, AppState};
use crate::models::{
    media::{Media, MediaOrigin},
    playback::PlaybackStatus,
    playlist::{adjacent_entry, PlaylistItemView, PlaylistSnapshot},
};
use crate::services::{media_assets, playback_plan};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

pub fn playlist_snapshot(state: &AppState) -> Result<PlaylistSnapshot, String> {
    let (revision, entries) = {
        let database = lock(&state.database)?;
        let directory = database.directory()?;
        let mut entries = Vec::new();
        for entry in database.playlist()? {
            let mut media = database.media(&entry.media_id)?;
            if matches!(media.origin, MediaOrigin::Remote { .. }) {
                for asset in std::mem::take(&mut media.assets) {
                    if crate::services::directory_library::contains_path(
                        &directory.path,
                        &asset.path,
                    )? {
                        media.assets.push(asset);
                    }
                }
            }
            entries.push((entry, media));
        }
        (
            state
                .playlist_snapshot_version
                .fetch_add(1, Ordering::SeqCst)
                + 1,
            entries,
        )
    };
    // File availability may be slow on a network volume; do not hold database/job locks.
    let items = entries
        .into_iter()
        .map(|(entry, media)| PlaylistItemView::new(entry, media))
        .collect();
    Ok(PlaylistSnapshot { revision, items })
}

pub async fn add_local_paths(
    state: AppState,
    app: AppHandle,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let task_state = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock(&task_state.directory_operation)?;
        let directory = lock(&task_state.database)?.directory()?;
        let media = paths
            .iter()
            .map(|path| media_assets::local_media(path))
            .collect::<Result<Vec<_>, _>>()?;
        for item in &media {
            if !crate::services::directory_library::contains_path(
                &directory.path,
                item.local_path().ok_or("Expected a local file")?,
            )? {
                return Err("文件不在当前保存目录中，请先在设置中切换目录".into());
            }
        }
        lock(&task_state.database)?.add_to_playlist(&media)?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| error.to_string())??;
    app.emit("playlist-updated", ())
        .map_err(|error| error.to_string())
}

pub async fn play_entry(state: AppState, app: AppHandle, entry_id: String) -> Result<(), String> {
    let id = {
        // When two locks are needed, playback always precedes database.
        let mut playback = lock(&state.playback)?;
        let media = lock(&state.database)?.entry_media(&entry_id)?;
        playback.begin(media, Some(entry_id))?
    };
    prepare_session(state, app, id).await
}

pub async fn play_path(state: AppState, app: AppHandle, path: PathBuf) -> Result<(), String> {
    // Reserve user intent before filesystem work, which may block on a network drive.
    let id = lock(&state.playback)?.reserve()?;
    let result =
        tauri::async_runtime::spawn_blocking(move || media_assets::local_media(&path)).await;
    if !lock(&state.playback)?.is_requested(id) {
        return Ok(());
    }
    let draft = result.map_err(|error| error.to_string())??;
    {
        let mut playback = lock(&state.playback)?;
        if !playback.is_requested(id) {
            return Ok(());
        }
        let media = lock(&state.database)?.register_media(&draft)?;
        if !playback.begin_reserved(id, media, None)? {
            return Ok(());
        }
    }
    prepare_session(state, app, id).await
}

async fn prepare_session(state: AppState, app: AppHandle, id: u64) -> Result<(), String> {
    app.emit("player-state-changed", ())
        .map_err(|error| error.to_string())?;
    let result = prepare_current(&state, &app, id).await;
    if let Err(error) = &result {
        lock(&state.playback)?.fail(id, error.clone());
    }
    app.emit("player-state-changed", ())
        .map_err(|error| error.to_string())?;
    // Obsolete completions do not report an error against the new selection.
    if !lock(&state.playback)?.matches(id) {
        return Ok(());
    }
    result
}

async fn prepare_current(state: &AppState, app: &AppHandle, id: u64) -> Result<(), String> {
    let media = {
        let playback = lock(&state.playback)?;
        let Some(session) = playback.session.as_ref().filter(|session| session.id == id) else {
            return Ok(());
        };
        session.media.clone()
    };
    let path = media
        .local_path()
        .ok_or("媒体尚未下载，请在下载窗口创建任务")?;
    if !path.is_file() {
        return Err(format!("媒体文件不可用：{}", path.display()));
    }
    if matches!(media.origin, MediaOrigin::Remote { .. }) {
        let directory = lock(&state.database)?.directory()?;
        if !crate::services::directory_library::contains_path(&directory.path, path)? {
            return Err("媒体文件不在当前保存目录中，请刷新播放列表".into());
        }
    }
    if !lock(&state.playback)?.matches(id) {
        return Ok(());
    }
    let media = lock(&state.database)?.media(&media.id)?;
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let subtitles = media_assets::local_subtitles(&media)?;
        let plan = playback_plan::prepare(&media)?;
        Ok::<_, String>((media, subtitles, plan))
    })
    .await
    .map_err(|error| error.to_string())??;
    let (mut media, subtitles, plan) = prepared;
    // A file operation may stop this session while probing. Fence stale work
    // before it can restore subtitle paths or activate the old filename.
    let _operation = lock(&state.directory_operation)?;
    let mut playback = lock(&state.playback)?;
    if !playback.matches(id) {
        return Ok(());
    }
    if !subtitles.is_empty() {
        media = lock(&state.database)?.attach_assets(&media.id, &subtitles)?;
    }
    let kind = match plan {
        crate::models::playback::PlaybackPlan::Audio { .. } => {
            crate::models::media::MediaType::Audio
        }
        _ => crate::models::media::MediaType::Video,
    };
    if media.media_type != kind {
        lock(&state.database)?.set_media_type(&media.id, &kind)?;
        media.media_type = kind;
        app.emit("playlist-updated", ())
            .map_err(|error| error.to_string())?;
    }
    playback.activate(id, media, plan)
}

pub fn remove_entries(state: &AppState, entry_id: Option<&str>) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let mut playback = lock(&state.playback)?;
    lock(&state.database)?.remove_entry(entry_id)?;
    let stop = entry_id.is_none()
        || playback
            .session
            .as_ref()
            .is_some_and(|session| session.playlist_entry_id.as_deref() == entry_id);
    if stop {
        playback
            .stop()
            .map_err(|error| format!("Playlist updated, but failed to stop playback: {error}"))?;
    }
    Ok(())
}

pub async fn advance(
    state: AppState,
    app: AppHandle,
    session_id: u64,
    backwards: bool,
    automatic: bool,
) -> Result<(), String> {
    let mode = lock(&state.settings)?.play_mode.clone();
    let id = {
        let mut playback = lock(&state.playback)?;
        playback.snapshot();
        if !playback.matches(session_id) {
            return Ok(());
        }
        let Some(session) = playback
            .session
            .as_ref()
            .filter(|session| session.id == session_id)
        else {
            return Ok(());
        };
        if automatic
            && (session.status != PlaybackStatus::Ended || session.playlist_entry_id.is_none())
        {
            return Ok(());
        }
        let database = lock(&state.database)?;
        let entries = database.playlist()?;
        let Some(entry_id) = adjacent_entry(
            &entries,
            session.playlist_entry_id.as_deref(),
            &mode,
            backwards,
        ) else {
            return Ok(());
        };
        playback.begin(database.entry_media(&entry_id)?, Some(entry_id))?
    };
    prepare_session(state, app, id).await
}

pub fn remote_media(
    url: String,
    title: String,
    external_id: String,
    media_type: crate::models::media::MediaType,
) -> Result<Media, String> {
    let provider = crate::models::media::provider_key_for_url(&url)?;
    Ok(Media {
        id: uuid::Uuid::new_v4().to_string(),
        canonical_key: crate::models::media::canonical_remote_key(&provider, &external_id),
        title,
        media_type,
        origin: MediaOrigin::Remote {
            url,
            provider,
            external_id,
        },
        assets: Vec::new(),
    })
}
