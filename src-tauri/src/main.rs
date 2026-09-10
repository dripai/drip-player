#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod handlers;
mod models;
mod services;
mod utils;

use handlers::settings::{
    apply_close_behavior, get_app_settings, open_settings_window, show_app_context_menu,
    update_app_settings, SettingsWindowState, TraySettingsItem,
};
use models::player_state::{MusicPlayer, PlayerState};
use models::playlist::{
    canonical_local_identity, canonical_remote_key, provider_key_for_url, LibraryItem,
    LibrarySource, MediaType, PlaylistItem, PlaylistOrigin, PlaylistSnapshot,
};
use services::media_capabilities;
use services::media_probe::{self, MediaInfo};
use services::online_resolver::{OnlineResolver, VideoPlatform};
use services::playback_plan::{self, PlaybackPlan};
use services::toolchain;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{
    menu::{CheckMenuItem, ContextMenu, Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, Window,
};
use tauri_plugin_dialog::DialogExt;

use std::ops::Deref;
use std::path::Path;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 创建一个在 Windows 上隐藏控制台窗口的命令
#[cfg(windows)]
fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}
#[cfg(not(windows))]
fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

#[derive(Clone)]
struct AppState(Arc<Mutex<MusicPlayer>>);

/// 获取可执行文件目录中的缓存目录
fn get_cache_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
        .join("cache")
}

/// 获取下载临时目录
fn get_download_dir() -> std::path::PathBuf {
    get_cache_dir().join("downloading")
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum AddUrlOutcome {
    Added,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct AddUrlResult {
    outcome: AddUrlOutcome,
    item_id: String,
}

impl Deref for AppState {
    type Target = Arc<Mutex<MusicPlayer>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn now_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn replace_playlist_items(
    player: &mut MusicPlayer,
    items: Vec<PlaylistItem>,
) -> Result<(), String> {
    let view_revision = player
        .playlist_revision
        .checked_add(1)
        .ok_or_else(|| "Playlist view revision overflow".to_string())?;
    let stored = player
        .persistence
        .replace_playlist(player.playlist_storage_revision, items)?;
    player.playlist_items = stored.items;
    player.playlist_storage_revision = stored.revision;
    player.playlist_revision = view_revision;
    Ok(())
}

fn item_cache_dir(item: &PlaylistItem) -> Option<std::path::PathBuf> {
    let PlaylistOrigin::Remote {
        provider,
        external_id,
        ..
    } = &item.origin
    else {
        return None;
    };
    let safe_provider = OnlineResolver::sanitize_filename(provider);
    let safe_external_id = OnlineResolver::sanitize_filename(external_id);
    Some(
        get_cache_dir()
            .join("media")
            .join(safe_provider)
            .join(safe_external_id),
    )
}

fn find_cached_media(item: &PlaylistItem) -> Option<std::path::PathBuf> {
    let PlaylistOrigin::Remote { external_id, .. } = &item.origin else {
        return None;
    };
    item.cached_path
        .as_ref()
        .filter(|path| path.exists())
        .cloned()
        .or_else(|| {
            item_cache_dir(item).and_then(|dir| {
                OnlineResolver::find_existing_media(
                    &dir,
                    external_id,
                    &item.title,
                    &item.media_type,
                )
            })
        })
        .or_else(|| {
            let legacy_name = item.cached_path.as_ref().and_then(|path| path.file_name());
            let mut matches = std::fs::read_dir(get_cache_dir())
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && media_capabilities::is_supported_media_path(path)
                        && (legacy_name
                            .map(|name| path.file_name() == Some(name))
                            .unwrap_or(false)
                            || path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .contains(external_id))
                })
                .collect::<Vec<_>>();
            matches.sort();
            matches.into_iter().next()
        })
}

fn set_item_cached_path(
    items: &mut [PlaylistItem],
    item_id: &str,
    cached_path: std::path::PathBuf,
) -> bool {
    let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
        return false;
    };
    item.cached_path = Some(cached_path);
    true
}

fn reconcile_playlist_cache_state(player: &mut MusicPlayer) -> Result<bool, String> {
    let mut items = player.playlist_items.clone();
    let mut changed = false;
    for item in &mut items {
        if matches!(item.origin, PlaylistOrigin::Remote { .. }) {
            let cached_path = find_cached_media(item);
            if item.cached_path != cached_path {
                item.cached_path = cached_path;
                changed = true;
            }
        }
    }
    if changed {
        replace_playlist_items(player, items)?;
    }
    Ok(changed)
}

fn media_type_for_library_path(path: &Path) -> MediaType {
    media_probe::probe(path)
        .map(|info| info.media_type)
        .unwrap_or_else(|| media_capabilities::media_type_from_path(path))
}

fn build_local_playlist_item(path: &Path) -> Result<PlaylistItem, String> {
    if !path.exists() {
        return Err(format!(
            "Local media file does not exist: {}",
            path.display()
        ));
    }
    if !path.is_file() || !media_capabilities::is_supported_media_path(path) {
        return Err(format!("Unsupported local media file: {}", path.display()));
    }
    let (path, canonical_key) = canonical_local_identity(path);
    let title = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(PlaylistItem {
        id: uuid::Uuid::new_v4().to_string(),
        media_id: uuid::Uuid::new_v4().to_string(),
        canonical_key,
        title,
        media_type: media_type_for_library_path(&path),
        origin: PlaylistOrigin::Local { path },
        cached_path: None,
        added_at: now_timestamp(),
    })
}

fn add_local_paths_to_playlist(
    player: &mut MusicPlayer,
    paths: impl IntoIterator<Item = std::path::PathBuf>,
) -> Result<bool, String> {
    let mut items = player.playlist_items.clone();
    let mut changed = false;
    for path in paths {
        let item = build_local_playlist_item(&path)?;
        if items
            .iter()
            .any(|existing| existing.canonical_key == item.canonical_key)
        {
            continue;
        }
        items.push(item);
        changed = true;
    }
    if changed {
        replace_playlist_items(player, items)?;
    }
    Ok(changed)
}

/// 获取后端已经解析好的播放列表快照。
#[tauri::command]
fn get_playlist(state: State<AppState>) -> PlaylistSnapshot {
    let player = state.0.lock().unwrap();
    player.playlist_snapshot()
}

#[tauri::command]
fn get_playback_plan(item: LibraryItem) -> Result<PlaybackPlan, String> {
    playback_plan::plan_for_item(&item)
}

#[tauri::command]
fn probe_media(path: String) -> Result<MediaInfo, String> {
    let path = std::path::PathBuf::from(path);
    media_probe::probe(&path).ok_or_else(|| format!("Failed to probe media: {}", path.display()))
}

/// 获取播放器状态（播放/暂停、进度、时长、当前曲目）
#[tauri::command]
fn get_state(state: State<AppState>) -> PlayerState {
    let mut player = state.0.lock().unwrap();

    // 动态计算当前进度
    if player.is_playing {
        if let Some(start) = player.playback_start {
            let elapsed = start.elapsed();
            let total_elapsed = player.playback_offset + elapsed;

            // 根据媒体类型获取时长
            let duration = match player.current_media_type {
                Some(MediaType::Video) => player.duration, // 视频使用存储的时长
                _ => {
                    let d = player.audio.get_duration();
                    if d.as_secs_f32() > 0.0 {
                        player.duration = d;
                    }
                    player.duration
                }
            };

            if duration.as_secs_f32() > 0.0 {
                player.progress = total_elapsed.as_secs_f32() / duration.as_secs_f32();
                if player.progress > 1.0 {
                    player.progress = 1.0;
                }
            }
        }
    } else {
        // 如果暂停，仅使用存储的偏移量
        let duration = match player.current_media_type {
            Some(MediaType::Video) => player.duration,
            _ => {
                let d = player.audio.get_duration();
                if d.as_secs_f32() > 0.0 {
                    player.duration = d;
                }
                player.duration
            }
        };

        if duration.as_secs_f32() > 0.0 {
            player.progress = player.playback_offset.as_secs_f32() / duration.as_secs_f32();
        }
    }

    let current_item = if let Some(item_id) = &player.current_playlist_item_id {
        player
            .playlist_items
            .iter()
            .find(|item| &item.id == item_id)
            .map(PlaylistItem::to_library_item)
    } else {
        player.temporary_item.clone()
    };
    PlayerState {
        is_playing: player.is_playing,
        progress: player.progress,
        duration: player.duration.as_secs_f64(),
        current_item_id: player.current_playlist_item_id.clone(),
        current_item,
    }
}

fn apply_playback_plan(
    plan: PlaybackPlan,
    player: &mut MusicPlayer,
    player_handle: Arc<Mutex<MusicPlayer>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let media_type = plan.media_type();
    let local_path = plan.local_path().map(|p| p.to_path_buf());

    player.current_media_type = Some(media_type.clone());
    player.current_media_path = local_path.clone();
    player.duration = Duration::from_secs(0);

    if let Some(ref path) = local_path {
        if let Some(duration) = media_probe::duration(path) {
            player.duration = duration;
        }
    }

    match plan {
        PlaybackPlan::Audio { path } => {
            let rx = player.audio.play_file(path);
            player.is_playing = true;
            player.playback_start = Some(Instant::now());

            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                match rx.recv() {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        println!("Playback error: {}", e);
                        let mut player = player_handle.lock().unwrap();
                        player.is_playing = false;
                        player.playback_start = None;
                        drop(player);
                        let _ = app_handle_clone.emit("playback-error", e);
                        let _ = app_handle_clone.emit("player-state-changed", ());
                    }
                    Err(_) => {}
                }
            });
        }
        PlaybackPlan::ExternalVideo { path } => {
            let mpv_path = OnlineResolver::get_mpv_path().ok_or_else(|| {
                format!(
                    "MPV not found in {}",
                    toolchain::diagnostic_lib_dir().display()
                )
            })?;
            let child = Command::new(&mpv_path)
                .arg(&path)
                .arg("--force-window=yes")
                .arg("--title=Drip Player")
                .arg("--osd-level=1")
                .spawn()
                .map_err(|e| format!("Failed to start MPV: {}", e))?;

            player.video_process = Some(child);
            player.is_playing = true;
            player.playback_start = Some(Instant::now());
        }
        PlaybackPlan::BrowserVideo { .. } | PlaybackPlan::RemotePending { .. } => {
            player.is_playing = true;
            player.playback_start = Some(Instant::now());
        }
    }

    Ok(())
}

/// 按稳定 ID 播放列表项。
#[tauri::command]
async fn play_item(
    item_id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    let item = player
        .playlist_items
        .iter()
        .find(|item| item.id == item_id)
        .cloned()
        .ok_or_else(|| format!("Playlist item not found: {item_id}"))?
        .to_library_item();
    let plan = playback_plan::plan_for_item(&item)?;

    // 停止现有的播放
    player.audio.stop();
    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }

    // 重置播放状态
    player.playback_start = None;
    player.playback_offset = Duration::from_secs(0);
    player.progress = 0.0;
    player.temporary_item = None; // 清除临时项
    player.current_playlist_item_id = Some(item_id);
    apply_playback_plan(plan, &mut player, state.0.clone(), app_handle.clone())?;

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 直接播放曲目（不加入播放列表）
#[tauri::command]
async fn play_track_directly(
    item: LibraryItem,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    // 停止现有的播放
    player.audio.stop();
    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }

    // 重置播放状态
    player.playback_start = None;
    player.playback_offset = Duration::from_secs(0);
    player.progress = 0.0;

    // 设置临时项并清除当前播放列表项
    player.current_playlist_item_id = None;
    player.temporary_item = Some(item.clone());
    let plan = playback_plan::plan_for_item(&item)?;
    apply_playback_plan(plan, &mut player, state.0.clone(), app_handle.clone())?;

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 暂停播放
#[tauri::command]
fn pause(state: State<AppState>, app_handle: AppHandle) {
    let mut player = state.0.lock().unwrap();
    player.audio.pause();
    player.is_playing = false;

    // 更新偏移量
    if let Some(start) = player.playback_start {
        player.playback_offset += start.elapsed();
        player.playback_start = None;
    }

    app_handle.emit("player-state-changed", ()).unwrap();
}

/// 恢复播放
#[tauri::command]
fn resume(state: State<AppState>, app_handle: AppHandle) {
    let mut player = state.0.lock().unwrap();
    if player.is_playing {
        return;
    }
    player.audio.resume();
    player.is_playing = true;

    // 重新开始追踪
    player.playback_start = Some(Instant::now());

    app_handle.emit("player-state-changed", ()).unwrap();
}

/// 跳转到指定进度 (0.0 - 1.0)
#[tauri::command]
fn seek(progress: f32, state: State<AppState>) {
    let mut player = state.0.lock().unwrap();
    let duration = player.duration;
    let seek_time = Duration::from_secs_f32(duration.as_secs_f32() * progress);

    // 更新所有媒体类型的进度追踪
    player.playback_offset = seek_time;
    player.progress = progress;
    if player.is_playing {
        player.playback_start = Some(Instant::now());
    } else {
        player.playback_start = None;
    }

    // 对于音频，也在后端进行 seek
    if let Some(MediaType::Audio) = player.current_media_type {
        player.audio.seek(seek_time);
    }
}

/// 设置音量 (0.0 - 1.0)
#[tauri::command]
fn set_volume(volume: f32, state: State<AppState>) {
    let mut player = state.0.lock().unwrap();
    player.volume = volume;
    player.audio.set_volume(volume);
}

#[tauri::command]
async fn add_local_files(
    paths: Vec<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let paths = paths.into_iter().map(std::path::PathBuf::from);
    let mut player = state.0.lock().unwrap();
    if add_local_paths_to_playlist(&mut player, paths)? {
        app_handle.emit("playlist-updated", ()).unwrap();
    }
    Ok(())
}

/// 打开文件选择对话框并添加本地文件
#[tauri::command]
async fn pick_and_add_local_files(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let file_paths = app_handle
        .dialog()
        .file()
        .add_filter("Media Files", media_capabilities::MEDIA_EXTENSIONS)
        .blocking_pick_files();

    if let Some(paths) = file_paths {
        let paths = paths
            .into_iter()
            .map(|path| {
                path.into_path()
                    .map_err(|_| "Failed to read selected path".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut player = state.0.lock().unwrap();
        if add_local_paths_to_playlist(&mut player, paths)? {
            app_handle.emit("playlist-updated", ()).unwrap();
        }
    }

    Ok(())
}

/// 打开文件夹选择对话框并递归添加媒体文件
#[tauri::command]
async fn pick_and_add_folder(
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let folder_path = app_handle.dialog().file().blocking_pick_folder();

    if let Some(folder) = folder_path {
        let folder_buf = match folder.into_path() {
            Ok(p) => p,
            Err(_) => return Err("Failed to get folder path".into()),
        };

        fn collect_media_paths(
            dir: &Path,
            paths: &mut Vec<std::path::PathBuf>,
        ) -> Result<(), String> {
            let entries = std::fs::read_dir(dir)
                .map_err(|error| format!("Failed to read folder {}: {error}", dir.display()))?;
            for entry in entries {
                let path = entry
                    .map_err(|error| format!("Failed to read folder entry: {error}"))?
                    .path();
                if path.is_dir() {
                    collect_media_paths(&path, paths)?;
                } else if media_capabilities::is_supported_media_path(&path) {
                    paths.push(path);
                }
            }
            Ok(())
        }

        let mut paths = Vec::new();
        collect_media_paths(&folder_buf, &mut paths)?;
        if paths.is_empty() {
            return Err("No media files found in the selected folder".into());
        }
        paths.sort();
        let mut player = state.0.lock().unwrap();
        if add_local_paths_to_playlist(&mut player, paths)? {
            app_handle.emit("playlist-updated", ()).unwrap();
        }
    }

    Ok(())
}

/// 获取文件夹树结构
#[tauri::command]
fn get_folder_tree(folder_path: String) -> Result<LibraryItem, String> {
    let path = std::path::PathBuf::from(folder_path);

    fn scan_directory_tree(dir: &std::path::Path) -> Option<LibraryItem> {
        let folder_name = dir.file_name()?.to_string_lossy().to_string();
        let mut children = Vec::new();

        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| {
                let path = e.path();
                (
                    !path.is_dir(),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
            });

            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(subfolder) = scan_directory_tree(&path) {
                        children.push(subfolder);
                    }
                } else if path.is_file() {
                    if media_capabilities::is_supported_media_path(&path) {
                        let id = path.to_string_lossy().to_string();
                        let title = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        children.push(LibraryItem::Track {
                            id,
                            title,
                            media_type: media_type_for_library_path(&path),
                            source: crate::models::playlist::LibrarySource::Local {
                                path: path.clone(),
                            },
                            parent: path.parent().map(|p| p.to_path_buf()),
                        });
                    }
                }
            }
        }

        if children.is_empty() {
            None
        } else {
            Some(LibraryItem::Folder {
                name: folder_name,
                path: dir.to_path_buf(),
                children,
            })
        }
    }

    scan_directory_tree(&path).ok_or_else(|| "No media files found".to_string())
}

#[tauri::command]
async fn add_url_for_download(
    url: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<AddUrlResult, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    {
        let player = state.0.lock().unwrap();
        if let Some(item) = player.playlist_items.iter().find(|item| {
            matches!(&item.origin, PlaylistOrigin::Remote { url: existing, .. } if existing == &url)
        }) {
            return Ok(AddUrlResult {
                outcome: AddUrlOutcome::AlreadyPresent,
                item_id: item.id.clone(),
            });
        }
    }

    app_handle.emit("url-resolving", true).unwrap();
    let url_for_metadata = url.clone();
    let metadata_result = tauri::async_runtime::spawn_blocking(move || {
        OnlineResolver::resolve_metadata(&url_for_metadata)
    })
    .await;
    app_handle.emit("url-resolving", false).unwrap();

    let metadata = metadata_result
        .map_err(|error| format!("URL metadata task failed: {error}"))?
        .map_err(|error| format!("Failed to resolve URL: {error}"))?;
    let provider = provider_key_for_url(&url);
    let canonical_key = canonical_remote_key(&provider, &metadata.id);

    let mut player = state.0.lock().unwrap();
    if let Some(item) = player
        .playlist_items
        .iter()
        .find(|item| item.canonical_key == canonical_key)
    {
        return Ok(AddUrlResult {
            outcome: AddUrlOutcome::AlreadyPresent,
            item_id: item.id.clone(),
        });
    }

    let media_type = metadata.get_media_type();
    let mut item = PlaylistItem {
        id: uuid::Uuid::new_v4().to_string(),
        media_id: uuid::Uuid::new_v4().to_string(),
        canonical_key,
        title: metadata.title,
        media_type,
        origin: PlaylistOrigin::Remote {
            url,
            provider,
            external_id: metadata.id,
        },
        cached_path: None,
        added_at: now_timestamp(),
    };
    item.cached_path = find_cached_media(&item);
    let item_id = item.id.clone();
    let mut items = player.playlist_items.clone();
    items.push(item);
    replace_playlist_items(&mut player, items)?;
    drop(player);

    app_handle.emit("playlist-updated", ()).unwrap();
    Ok(AddUrlResult {
        outcome: AddUrlOutcome::Added,
        item_id,
    })
}

fn finalize_downloaded_media(
    temp_path: &Path,
    cache_dir: &Path,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("Failed to create media cache directory: {error}"))?;
    let extension = temp_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("bin");
    let final_path = cache_dir.join(format!("media.{extension}"));

    if final_path.exists() {
        if temp_path != final_path {
            let _ = std::fs::remove_file(temp_path);
        }
        return Ok(final_path);
    }

    std::fs::rename(temp_path, &final_path)
        .or_else(|_| {
            std::fs::copy(temp_path, &final_path)?;
            std::fs::remove_file(temp_path)
        })
        .map_err(|error| format!("Failed to move downloaded media into cache: {error}"))?;
    Ok(final_path)
}

fn move_downloaded_subtitles(download_dir: &Path, cache_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(download_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase);
        if !matches!(extension.as_deref(), Some("srt" | "vtt" | "ass" | "ssa")) {
            continue;
        }
        let target = cache_dir.join(path.file_name().unwrap_or_default());
        if target.exists() {
            let _ = std::fs::remove_file(path);
            continue;
        }
        let _ = std::fs::rename(&path, &target).or_else(|_| {
            std::fs::copy(&path, &target)?;
            std::fs::remove_file(&path)
        });
    }
}

#[tauri::command]
async fn download_and_play(
    item_id: String,
    extra_subtitle_lang: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut item = {
        let player = state.0.lock().unwrap();
        player
            .playlist_items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| format!("Playlist item not found: {item_id}"))?
    };
    let PlaylistOrigin::Remote {
        url, external_id, ..
    } = &item.origin
    else {
        return Err("Selected playlist item is not remote media".to_string());
    };

    if let Some(cached_path) = find_cached_media(&item) {
        if item.cached_path.as_ref() != Some(&cached_path) {
            {
                let mut player = state.0.lock().unwrap();
                let mut items = player.playlist_items.clone();
                if !set_item_cached_path(&mut items, &item_id, cached_path.clone()) {
                    return Err(format!("Playlist item not found: {item_id}"));
                }
                replace_playlist_items(&mut player, items)?;
            }
            item.cached_path = Some(cached_path);
            app_handle.emit("playlist-updated", ()).unwrap();
        }
        return play_item(item_id, state, app_handle).await;
    }

    let download_dir = get_download_dir().join(&item_id);
    let cache_dir = item_cache_dir(&item)
        .ok_or_else(|| "Remote playlist item has no cache identity".to_string())?;
    std::fs::create_dir_all(&download_dir)
        .map_err(|error| format!("Failed to create download directory: {error}"))?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("Failed to create cache directory: {error}"))?;

    {
        let mut player = state.0.lock().unwrap();
        if !player.playlist_items.iter().any(|item| item.id == item_id) {
            return Err(format!("Playlist item not found: {item_id}"));
        }
        if !player.downloading_item_ids.insert(item_id.clone()) {
            return Err("This media item is already downloading".to_string());
        }
        player.playlist_revision = player.playlist_revision.saturating_add(1);
    }
    app_handle.emit("playlist-updated", ()).unwrap();

    let url = url.clone();
    let external_id = external_id.clone();
    let title = item.title.clone();
    let media_type = item.media_type.clone();
    let progress_handle = app_handle.clone();
    let download_dir_for_task = download_dir.clone();
    let download_result = match tokio::task::spawn_blocking(move || {
        OnlineResolver::download_media(
            &url,
            &external_id,
            &title,
            &download_dir_for_task,
            media_type,
            extra_subtitle_lang.as_deref(),
            move |progress| {
                let _ = progress_handle.emit("download-progress", progress);
            },
        )
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(format!("Download task failed: {error}")),
    };

    match download_result {
        Ok(temp_path) => {
            let final_path = match finalize_downloaded_media(&temp_path, &cache_dir) {
                Ok(path) => path,
                Err(error) => {
                    let mut player = state.0.lock().unwrap();
                    if player.downloading_item_ids.remove(&item_id) {
                        player.playlist_revision = player.playlist_revision.saturating_add(1);
                    }
                    drop(player);
                    app_handle.emit("playlist-updated", ()).unwrap();
                    return Err(error);
                }
            };
            move_downloaded_subtitles(&download_dir, &cache_dir);

            let update_result = {
                let mut player = state.0.lock().unwrap();
                player.downloading_item_ids.remove(&item_id);
                let mut items = player.playlist_items.clone();
                if set_item_cached_path(&mut items, &item_id, final_path) {
                    replace_playlist_items(&mut player, items).map(|_| true)
                } else {
                    Ok(false)
                }
            };
            let item_still_exists = match update_result {
                Ok(exists) => exists,
                Err(error) => {
                    app_handle.emit("playlist-updated", ()).unwrap();
                    return Err(error);
                }
            };
            if !item_still_exists {
                app_handle.emit("playlist-updated", ()).unwrap();
                return Err("Playlist item was removed while downloading".to_string());
            }
            app_handle.emit("playlist-updated", ()).unwrap();
            play_item(item_id, state, app_handle).await
        }
        Err(error) => {
            let mut player = state.0.lock().unwrap();
            if player.downloading_item_ids.remove(&item_id) {
                player.playlist_revision = player.playlist_revision.saturating_add(1);
            }
            drop(player);
            app_handle.emit("playlist-updated", ()).unwrap();
            Err(format!("Download failed: {error}"))
        }
    }
}

/// 播放出错时的处理 (尝试使用后端播放器)
#[tauri::command]
async fn on_playback_error(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    println!("Frontend playback failed");

    // 获取当前曲目信息。
    let (url_or_path, is_video) = {
        let player = state.0.lock().unwrap();
        let current_item = player
            .current_playlist_item_id
            .as_ref()
            .and_then(|item_id| {
                player
                    .playlist_items
                    .iter()
                    .find(|item| &item.id == item_id)
                    .map(PlaylistItem::to_library_item)
            })
            .or_else(|| player.temporary_item.clone());
        let Some(LibraryItem::Track {
            source, media_type, ..
        }) = current_item
        else {
            return Ok(());
        };
        let url = match source {
            LibrarySource::Local { path } => path.to_string_lossy().to_string(),
            LibrarySource::Remote {
                url, cached_path, ..
            } => cached_path
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or(url),
        };
        (url, media_type == MediaType::Video)
    };

    // 仅对音频文件使用 ffplay，或作为视频的最后手段
    // 对于视频，前端视频播放器应该处理它
    if !is_video {
        // 尝试使用后端音频播放器播放音频文件
        if let Some(ffplay) = OnlineResolver::get_ffplay_path() {
            let play_target = if url_or_path.starts_with("http") {
                println!("Attempting to resolve stream URL for backend playback...");
                match OnlineResolver::get_stream_url(&url_or_path) {
                    Ok(u) => {
                        println!("Resolved stream URL: {}", u);
                        u
                    }
                    Err(e) => {
                        println!("Failed to resolve stream URL: {}. ", e);
                        if url_or_path.contains("bilibili.com") {
                            println!("Cannot play Bilibili webpage directly in backend player. Aborting.");
                            return Ok(());
                        }
                        url_or_path.clone()
                    }
                }
            } else {
                url_or_path.clone()
            };

            println!("Launching ffplay with: {}", play_target);

            let mut cmd = Command::new(ffplay);

            if play_target.starts_with("http") {
                cmd.arg("-headers");
                // 根据原始 URL 添加特定于平台的引用页
                let platform = VideoPlatform::from_url(&url_or_path);
                if let Some(referer) = platform.get_referer() {
                    cmd.arg(format!("Referer: {}\r\nUser-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36\r\n", referer));
                } else {
                    cmd.arg("User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36\r\n");
                }
            }

            let child_result = cmd
                .arg(&play_target)
                .arg("-autoexit")
                .arg("-window_title")
                .arg("Drip Music Player")
                .spawn();

            match child_result {
                Ok(c) => {
                    let mut player = state.0.lock().unwrap();
                    // 如果有现有进程，则杀死它
                    if let Some(mut child) = player.video_process.take() {
                        let _ = child.kill();
                    }
                    player.video_process = Some(c);
                    player.is_playing = true;
                    player.playback_start = Some(Instant::now());
                }
                Err(e) => {
                    println!("Failed to start ffplay: {}", e);
                }
            }
        } else {
            let lib_path =
                toolchain::diagnostic_lib_dir().join(toolchain::executable_name("ffplay"));
            println!("ffplay not found in bundled lib");
            println!("Checked lib path: {}", lib_path.display());
        }
    } else {
        println!("Video playback error - frontend should handle video playback");
    }

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 显示曲目上下文菜单
#[tauri::command]
async fn show_track_context_menu(
    window: Window,
    item_id: String,
    locale: String,
) -> Result<(), String> {
    let app_handle = window.app_handle().clone();

    let label = if locale == "zh" {
        "从播放列表移除"
    } else {
        "Remove from playlist"
    };

    let remove_item = MenuItem::with_id(
        &app_handle,
        format!("remove_item:{item_id}"),
        label,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(&app_handle, &[&remove_item]).map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}

/// 显示播放列表上下文菜单
#[tauri::command]
async fn show_playlist_context_menu(window: Window, locale: String) -> Result<(), String> {
    let app_handle = window.app_handle().clone();

    let (clear_playlist_label, clear_tree_label) = if locale == "zh" {
        ("清空播放列表", "清空文件夹树")
    } else {
        ("Clear entire playlist", "Clear folder tree")
    };

    let clear_playlist_item = MenuItem::with_id(
        &app_handle,
        "clear_playlist",
        clear_playlist_label,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let clear_tree_item = MenuItem::with_id(
        &app_handle,
        "clear_tree",
        clear_tree_label,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(&app_handle, &[&clear_playlist_item, &clear_tree_item])
        .map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}

fn remove_playlist_item(player: &mut MusicPlayer, item_id: &str) -> Result<bool, String> {
    if !player.playlist_items.iter().any(|item| item.id == item_id) {
        return Ok(false);
    }
    let mut items = player.playlist_items.clone();
    items.retain(|item| item.id != item_id);
    replace_playlist_items(player, items)?;
    player.downloading_item_ids.remove(item_id);

    if player.current_playlist_item_id.as_deref() == Some(item_id) {
        player.current_playlist_item_id = None;
        player.is_playing = false;
        player.audio.stop();
        if let Some(mut child) = player.video_process.take() {
            let _ = child.kill();
        }
    }
    Ok(true)
}

/// 按稳定 ID 移除曲目。
#[tauri::command]
async fn remove_track(
    item_id: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    if remove_playlist_item(&mut player, &item_id)? {
        drop(player);
        app_handle.emit("playlist-updated", ()).unwrap();
        app_handle.emit("player-state-changed", ()).unwrap();
    }
    Ok(())
}

/// 清空播放列表
#[tauri::command]
async fn clear_playlist(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    replace_playlist_items(&mut player, Vec::new())?;
    player.current_playlist_item_id = None;
    player.downloading_item_ids.clear();
    player.is_playing = false;
    player.audio.stop();

    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }
    drop(player);
    app_handle.emit("playlist-updated", ()).unwrap();
    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 检查外部依赖 (yt-dlp, ffmpeg, ffplay)
#[tauri::command]
fn check_dependencies() -> Result<serde_json::Value, String> {
    use crate::services::online_resolver::OnlineResolver;
    use serde_json::json;

    let (yt_dlp_cmd, ffmpeg_dir) = OnlineResolver::get_tools_paths();
    let ffplay_path = OnlineResolver::get_ffplay_path();

    // 检查 yt-dlp
    let yt_dlp_available = hidden_command(&yt_dlp_cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 检查 ffmpeg
    let ffmpeg_available = if ffmpeg_dir.is_some() {
        OnlineResolver::get_ffmpeg_path().is_some()
    } else {
        false
    };

    // 检查 ffplay
    let ffplay_available = ffplay_path.is_some();

    Ok(json!({
        "yt_dlp": {
            "available": yt_dlp_available,
            "path": yt_dlp_cmd
        },
        "ffmpeg": {
            "available": ffmpeg_available,
            "required": false,
            "purpose": "Video format conversion and merging"
        },
        "ffplay": {
            "available": ffplay_available,
            "required": false,
            "purpose": "External video player"
        }
    }))
}

/// 获取视频文件的 URL (本地路径转换)
#[tauri::command]
fn get_video_url(path: String) -> Result<String, String> {
    let path_buf = std::path::PathBuf::from(&path);

    if !path_buf.exists() {
        return Err(format!("File not found: {}", path));
    }

    // 原样返回路径，前端的 convertFileSrc 会处理它
    Ok(path)
}

/// 播放网络视频 (解析流地址)
#[tauri::command]
async fn play_online_video(window: Window, url: String) -> Result<(), String> {
    let platform = VideoPlatform::from_url(&url);
    println!("Resolving {} video URL: {}", platform.display_name(), url);

    let video_url =
        OnlineResolver::get_stream_url(&url).map_err(|e| format!("Failed to get video: {}", e))?;

    println!("Resolved video URL: {}", video_url);

    // 发送到前端
    window
        .emit("online_video_url", video_url)
        .map_err(|e| format!("Failed to emit event: {}", e))?;

    Ok(())
}

// 保持旧名称以向后兼容
/// 播放 Bilibili 视频 (兼容接口)
#[tauri::command]
async fn play_bilibili_video(window: Window, url: String) -> Result<(), String> {
    play_online_video(window, url).await
}

/// 使用外部 MPV 播放器播放
#[tauri::command]
async fn play_with_mpv(
    path: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    println!("Playing with MPV: {}", path);

    // 获取 MPV 路径
    let mpv_path = OnlineResolver::get_mpv_path().ok_or_else(|| {
        format!(
            "MPV not found in {}",
            toolchain::diagnostic_lib_dir().display()
        )
    })?;

    println!("Using MPV: {}", mpv_path);

    // 杀死现有的视频进程
    {
        let mut player = state.0.lock().unwrap();
        if let Some(mut child) = player.video_process.take() {
            let _ = child.kill();
        }
    }

    // 启动 MPV 进程
    let child = Command::new(&mpv_path)
        .arg(&path)
        .arg("--force-window=yes")
        .arg("--title=Drip Player")
        .arg("--osd-level=1")
        .spawn()
        .map_err(|e| format!("Failed to start MPV: {}", e))?;

    // 存储进程并更新状态
    {
        let mut player = state.0.lock().unwrap();
        player.video_process = Some(child);
        player.is_playing = true;
        player.playback_start = Some(Instant::now());
    }

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 检查 MPV 是否可用
#[tauri::command]
fn check_mpv_available() -> bool {
    OnlineResolver::get_mpv_path().is_some()
}

/// 在默认浏览器中打开平台的登录页面
#[tauri::command]
async fn open_platform_login(platform: String) -> Result<String, String> {
    let login_url = match platform.to_lowercase().as_str() {
        "youtube" => "https://accounts.google.com/ServiceLogin?service=youtube",
        "bilibili" | "哔哩哔哩" => "https://passport.bilibili.com/login",
        "douyin" | "抖音" => "https://www.douyin.com/login",
        "tencent" | "腾讯视频" => "https://v.qq.com/",
        "weixin" | "微信视频号" => "https://channels.weixin.qq.com/",
        _ => return Err(format!("Unknown platform: {}", platform)),
    };

    // 在默认浏览器中打开
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", login_url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(login_url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(login_url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    Ok(login_url.to_string())
}

/// 检查错误是否指示需要登录，并返回登录信息
#[tauri::command]
fn check_login_required(error: String) -> Option<LoginRequiredInfo> {
    if let Some((platform, login_url, message)) = OnlineResolver::parse_login_error(&error) {
        Some(LoginRequiredInfo {
            platform,
            login_url,
            message,
        })
    } else {
        None
    }
}

#[derive(serde::Serialize)]
struct LoginRequiredInfo {
    platform: String,
    login_url: String,
    message: String,
}

/// 尝试使用 OAuth2 认证添加 URL (针对 YouTube)
/// 这将触发基于浏览器的 OAuth 流程
#[tauri::command]
async fn add_url_with_oauth(url: String, window: Window) -> Result<(), String> {
    println!("Attempting to add URL with OAuth2: {}", url);

    // 发送解析状态
    window.emit("url-resolving", true).ok();

    let url_clone = url.clone();
    let result = tokio::task::spawn_blocking(move || {
        OnlineResolver::resolve_metadata_with_oauth(&url_clone)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?;

    window.emit("url-resolving", false).ok();

    match result {
        Ok(metadata) => {
            println!("OAuth2 resolved: {} ({})", metadata.title, metadata.id);
            // 现在使用常规流程添加到播放列表
            // 我们需要在这里调用 add_url_for_download 逻辑
            // 目前，发送成功信号并让前端使用正常流程重试
            window.emit("oauth-success", &url).ok();
            Ok(())
        }
        Err(e) => {
            println!("OAuth2 failed: {}", e);
            Err(e)
        }
    }
}

#[derive(serde::Serialize)]
struct SubtitleInfo {
    lang: String,
    path: String,
}

/// 扫描视频文件的字幕
#[tauri::command]
fn scan_subtitles(video_path: String) -> Vec<SubtitleInfo> {
    let video_path = std::path::Path::new(&video_path);
    let mut subtitles = Vec::new();

    // 获取视频的目录和文件名 (不含扩展名)
    let parent = match video_path.parent() {
        Some(p) => p,
        None => return subtitles,
    };

    let stem = match video_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return subtitles,
    };

    // 扫描具有相同文件名的字幕文件
    let subtitle_extensions = ["srt", "vtt", "ass", "ssa"];

    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if subtitle_extensions.contains(&ext_str.as_str()) {
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                        // 检查字幕文件是否匹配视频文件名
                        // 模式: stem.lang.ext 或 stem.ext
                        if file_name.starts_with(&format!("{}.", stem)) {
                            // 从文件名中提取语言
                            let name_without_ext =
                                path.file_stem().unwrap_or_default().to_string_lossy();
                            let lang = if name_without_ext.len() > stem.len() + 1 {
                                // 包含语言代码: stem.lang
                                name_without_ext[stem.len() + 1..].to_string()
                            } else {
                                // 没有语言代码，使用扩展名作为标识符
                                ext_str.to_uppercase()
                            };

                            subtitles.push(SubtitleInfo {
                                lang,
                                path: path.to_string_lossy().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    // 按语言排序
    subtitles.sort_by(|a, b| a.lang.cmp(&b.lang));
    subtitles
}

fn main() {
    let mut initial_player = MusicPlayer::new()
        .unwrap_or_else(|error| panic!("Failed to initialize application database: {error}"));
    reconcile_playlist_cache_state(&mut initial_player)
        .unwrap_or_else(|error| panic!("Failed to reconcile playlist cache: {error}"));
    let player = Arc::new(Mutex::new(initial_player));

    tauri::async_runtime::spawn_blocking(|| {
        services::media_remux::cleanup_remux_cache();
    });

    // 启动代理服务器
    tauri::async_runtime::spawn(async {
        services::stream_server::start_server(10001).await;
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState(player))
        .manage(SettingsWindowState::default())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let player = state.0.lock().unwrap();
                if player.settings.minimize_to_tray {
                    api.prevent_close();
                    window.hide().unwrap();
                } else if let Some(settings) = window.get_webview_window("settings") {
                    if let Err(error) = settings.destroy() {
                        api.prevent_close();
                        let _ = window.emit("app-error", error.to_string());
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
            let initial_minimize_to_tray = state.0.lock().unwrap().settings.minimize_to_tray;
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

                if let Some(label) = event_id.strip_prefix("app_refresh:") {
                    if let Some(window) = app.get_webview_window(label) {
                        if let Err(error) = window.eval("window.location.reload()") {
                            let _ = window.emit("app-error", error.to_string());
                        }
                    }
                    return;
                } else if let Some(locale) = event_id.strip_prefix("app_settings:") {
                    let app = app.clone();
                    let locale = locale.to_string();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = open_settings_window(app.clone(), locale).await {
                            let _ = app.emit("app-error", error);
                        }
                    });
                    return;
                } else if event_id == "tray_quit" {
                    app.exit(0);
                } else if event_id == "tray_restore" {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                } else if event_id == "tray_minimize_on_close" {
                    let enabled = !state_for_menu.0.lock().unwrap().settings.minimize_to_tray;
                    if let Err(error) = apply_close_behavior(app, enabled) {
                        let _ = app.emit("app-error", error);
                    }
                }
                if let Some(item_id) = event_id.strip_prefix("remove_item:") {
                    let item_id = item_id.to_string();
                    let state_clone = state_for_menu.clone();
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut player = state_clone.0.lock().unwrap();
                        match remove_playlist_item(&mut player, &item_id) {
                            Ok(true) => {
                                let _ = app_clone.emit("playlist-updated", ());
                                let _ = app_clone.emit("player-state-changed", ());
                            }
                            Ok(false) => {}
                            Err(error) => {
                                let _ = app_clone.emit("playback-error", error);
                            }
                        }
                    });
                } else if event_id == "clear_playlist" {
                    let state_clone = state_for_menu.clone();
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut player = state_clone.0.lock().unwrap();
                        if let Err(error) = replace_playlist_items(&mut player, Vec::new()) {
                            let _ = app_clone.emit("playback-error", error);
                            return;
                        }
                        player.current_playlist_item_id = None;
                        player.downloading_item_ids.clear();
                        player.is_playing = false;
                        player.audio.stop();
                        if let Some(mut child) = player.video_process.take() {
                            let _ = child.kill();
                        }
                        let _ = app_clone.emit("playlist-updated", ());
                        let _ = app_clone.emit("player-state-changed", ());
                    });
                } else if event_id == "clear_tree" {
                    let _ = app.emit("clear-folder-tree", ());
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_playlist,
            get_playback_plan,
            probe_media,
            play_item,
            play_track_directly,
            pause,
            resume,
            add_url_for_download,
            download_and_play,
            add_local_files,
            pick_and_add_local_files,
            pick_and_add_folder,
            get_folder_tree,
            show_track_context_menu,
            show_playlist_context_menu,
            show_app_context_menu,
            get_app_settings,
            update_app_settings,
            remove_track,
            clear_playlist,
            check_dependencies,
            get_video_url,
            seek,
            set_volume,
            get_state,
            on_playback_error,
            play_online_video,
            play_bilibili_video,
            play_with_mpv,
            check_mpv_available,
            open_platform_login,
            check_login_required,
            add_url_with_oauth,
            scan_subtitles
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_media_identity_does_not_depend_on_url_shape() {
        let long_url = "https://www.youtube.com/watch?v=ysaGeSbcnJA";
        let short_url = "https://youtu.be/ysaGeSbcnJA";
        let long_key = canonical_remote_key(&provider_key_for_url(long_url), "ysaGeSbcnJA");
        let short_key = canonical_remote_key(&provider_key_for_url(short_url), "ysaGeSbcnJA");
        assert_eq!(long_key, short_key);
    }

    #[test]
    fn playlist_item_keeps_title_when_remote_media_is_cached() {
        let item = PlaylistItem {
            id: "item-id".to_string(),
            media_id: "media-id".to_string(),
            canonical_key: "remote:youtube:video-id".to_string(),
            title: "Logical title".to_string(),
            media_type: MediaType::Video,
            origin: PlaylistOrigin::Remote {
                url: "https://youtu.be/video-id".to_string(),
                provider: "youtube".to_string(),
                external_id: "video-id".to_string(),
            },
            cached_path: Some(std::path::PathBuf::from("cache/media.mp4")),
            added_at: 1,
        };

        match item.to_library_item() {
            LibraryItem::Track { id, title, .. } => {
                assert_eq!(id, "item-id");
                assert_eq!(title, "Logical title");
            }
            _ => panic!("expected a track"),
        }
    }

    #[test]
    fn cache_update_targets_stable_id_after_another_item_is_removed() {
        let remote_item = |id: &str| PlaylistItem {
            id: id.to_string(),
            media_id: format!("media-{id}"),
            canonical_key: format!("remote:youtube:{id}"),
            title: id.to_string(),
            media_type: MediaType::Video,
            origin: PlaylistOrigin::Remote {
                url: format!("https://youtu.be/{id}"),
                provider: "youtube".to_string(),
                external_id: id.to_string(),
            },
            cached_path: None,
            added_at: 1,
        };
        let mut items = vec![remote_item("first"), remote_item("second")];
        items.retain(|item| item.id != "first");

        assert!(set_item_cached_path(
            &mut items,
            "second",
            std::path::PathBuf::from("cache/second.mp4"),
        ));
        assert_eq!(
            items[0].cached_path.as_deref(),
            Some(Path::new("cache/second.mp4"))
        );
    }
}
