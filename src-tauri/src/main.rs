#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

mod models;
mod services;
mod utils;
mod handlers;

use models::player_state::{MusicPlayer, PlayerState};
use models::playlist::{MediaType, TrackSource, Track, PlaylistItem};
use services::online_resolver::{OnlineResolver, VideoPlatform};
use services::persistence::PersistenceManager;
use std::sync::{Arc, Mutex};
use tauri::{State, AppHandle, Window, Emitter, Manager, menu::{Menu, MenuItem, ContextMenu}};
use std::time::{Duration, Instant};
use std::process::Command;
use tauri_plugin_dialog::DialogExt;

use std::ops::Deref;
use std::path::Path;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a Command that hides the console window on Windows
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

fn get_duration_with_ffprobe(path: &Path) -> Option<Duration> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let lib_dir = current_dir.join("lib");
    let ffprobe_in_lib = lib_dir.join("ffprobe.exe");

    let ffprobe_cmd = if ffprobe_in_lib.exists() {
        ffprobe_in_lib.to_string_lossy().to_string()
    } else {
        "ffprobe".to_string()
    };

    let output = hidden_command(&ffprobe_cmd)
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;

    if output.status.success() {
        let duration_str = String::from_utf8_lossy(&output.stdout);
        let duration_secs: f64 = duration_str.trim().parse().ok()?;
        println!("Duration from ffprobe: {} seconds for {:?}", duration_secs, path);
        Some(Duration::from_secs_f64(duration_secs))
    } else {
        println!("ffprobe failed for {:?}", path);
        None
    }
}

/// Get cache directory in executable's directory
fn get_cache_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
        .join("cache")
}

/// Get download temp directory
fn get_download_dir() -> std::path::PathBuf {
    get_cache_dir().join("downloading")
}

impl Deref for AppState {
    type Target = Arc<Mutex<MusicPlayer>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[tauri::command]
fn get_playlist(state: State<AppState>) -> Vec<Track> {
    let player = state.0.lock().unwrap();
    player.playlist.tracks.clone()
}

#[tauri::command]
fn get_state(state: State<AppState>) -> PlayerState {
    let mut player = state.0.lock().unwrap();

    // Calculate current progress dynamically
    if player.is_playing {
        if let Some(start) = player.playback_start {
            let elapsed = start.elapsed();
            let total_elapsed = player.playback_offset + elapsed;

            // Get duration based on media type
            let duration = match player.current_media_type {
                Some(MediaType::Video) => player.duration, // Use stored duration for video
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
                if player.progress > 1.0 { player.progress = 1.0; }
            }
        }
    } else {
        // If paused, just use stored offset
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

    let current_track = player.playlist.current_track().cloned();
    PlayerState {
        is_playing: player.is_playing,
        progress: player.progress,
        duration: player.duration.as_secs_f64(),
        current_index: player.playlist.current_index,
        current_track,
    }
}

#[tauri::command]
async fn play_track(index: usize, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    
    // Stop existing
    player.audio.stop();
    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }
    
    // Reset playback state
    player.playback_start = None;
    player.playback_offset = Duration::from_secs(0);
    player.progress = 0.0;

    if index >= player.playlist.tracks.len() {
        return Err("Index out of bounds".into());
    }

    player.playlist.current_index = Some(index);
    let track = player.playlist.tracks[index].clone();

    // Determine path and type
    let (path, media_type) = match &track.source {
        TrackSource::Local(p) => {
            let mt = if let Some(ext) = p.extension() {
                let s = ext.to_string_lossy().to_lowercase();
                // Only mp4/webm as video (browser supported)
                // mkv/avi/mov will be played as audio (extract audio track via ffmpeg)
                if ["mp4", "webm"].contains(&s.as_str()) {
                    MediaType::Video
                } else {
                    MediaType::Audio
                }
            } else {
                MediaType::Audio
            };
            (Some(p.clone()), mt)
        },
        TrackSource::Remote { cached_path, media_type, url, .. } => {
            if let Some(p) = cached_path {
                (Some(p.clone()), media_type.clone())
            } else {
                // Use URL as path for backend playback
                (Some(std::path::PathBuf::from(url)), media_type.clone())
            }
        }
    };

    player.current_media_type = Some(media_type.clone());

    if media_type == MediaType::Video {
        // Video handled by frontend - just update state
        player.is_playing = true;
        player.current_media_path = path.clone();
        player.playback_start = Some(Instant::now());

        // Get duration using ffprobe for video files
        if let Some(ref p) = path {
            if let Some(duration) = get_duration_with_ffprobe(p) {
                player.duration = duration;
            }
        }
    } else {
        // Audio handled by backend
        if let Some(p) = path {
            player.current_media_path = Some(p.clone());
            let rx = player.audio.play_file(p);
            player.is_playing = true;
            // Record start time
            player.playback_start = Some(Instant::now());

            // Handle result asynchronously to check for timeout/errors
            let app_handle_clone = app_handle.clone();
            let player_clone = state.0.clone();
            
            tauri::async_runtime::spawn(async move {
                match rx.recv() {
                    Ok(Ok(_)) => {
                        // Success
                    },
                    Ok(Err(e)) => {
                        println!("Playback error: {}", e);
                        // Update state to stopped
                        let mut player = player_clone.lock().unwrap();
                        player.is_playing = false;
                        player.playback_start = None;
                        drop(player); // Release lock
                        
                        let _ = app_handle_clone.emit("playback-error", e);
                        let _ = app_handle_clone.emit("player-state-changed", ());
                    },
                    Err(_) => {
                        // Channel closed unexpectedly
                    }
                }
            });
        } else {
             // Should not happen with above logic
             player.is_playing = true;
             player.current_media_path = None;
        }
    }
    
    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

#[tauri::command]
fn pause(state: State<AppState>, app_handle: AppHandle) {
    let mut player = state.0.lock().unwrap();
    player.audio.pause();
    player.is_playing = false;
    
    // Update offset
    if let Some(start) = player.playback_start {
        player.playback_offset += start.elapsed();
        player.playback_start = None;
    }
    
    app_handle.emit("player-state-changed", ()).unwrap();
}

#[tauri::command]
fn resume(state: State<AppState>, app_handle: AppHandle) {
    let mut player = state.0.lock().unwrap();
    if player.is_playing {
        return;
    }
    player.audio.resume();
    player.is_playing = true;
    
    // Start tracking again
    player.playback_start = Some(Instant::now());
    
    app_handle.emit("player-state-changed", ()).unwrap();
}

#[tauri::command]
fn seek(progress: f32, state: State<AppState>) {
    let mut player = state.0.lock().unwrap();
    let duration = player.duration;
    let seek_time = Duration::from_secs_f32(duration.as_secs_f32() * progress);

    // Update progress tracking for all media types
    player.playback_offset = seek_time;
    player.progress = progress;
    if player.is_playing {
        player.playback_start = Some(Instant::now());
    } else {
        player.playback_start = None;
    }

    // For audio, also seek in the backend
    if let Some(MediaType::Audio) = player.current_media_type {
        player.audio.seek(seek_time);
    }
}

#[tauri::command]
fn set_volume(volume: f32, state: State<AppState>) {
    let mut player = state.0.lock().unwrap();
    player.volume = volume;
    player.audio.set_volume(volume);
}

#[tauri::command]
async fn add_local_files(paths: Vec<String>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    let mut added = false;
    
    for path_str in paths {
        let path = std::path::PathBuf::from(path_str);
        // Basic validation
        if path.exists() {
             // Check if already exists
             let exists = player.playlist.tracks.iter().any(|t| {
                 match &t.source {
                     TrackSource::Local(p) => p.as_path() == path.as_path(),
                     _ => false
                 }
             });
             
             if !exists {
                 player.playlist.add_track(Track::new_local(path));
                 added = true;
             }
        }
    }
    
    if added {
        PersistenceManager::save_playlist(&player.playlist.tracks);
        app_handle.emit("playlist-updated", ()).unwrap();
    }
    Ok(())
}

#[tauri::command]
async fn pick_and_add_local_files(app_handle: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let file_paths = app_handle.dialog().file()
        .add_filter("Media Files", &["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mkv", "webm", "avi", "mov"])
        .blocking_pick_files();

    if let Some(paths) = file_paths {
        let mut player = state.0.lock().unwrap();
        let mut added = false;

        for path in paths {
            let path_buf = match path.into_path() {
                Ok(p) => p,
                Err(_) => continue,
            };

            if path_buf.exists() {
                 let exists = player.playlist.tracks.iter().any(|t| {
                     match &t.source {
                         TrackSource::Local(p) => p.as_path() == path_buf.as_path(),
                         _ => false
                     }
                 });

                 if !exists {
                     player.playlist.add_track(Track::new_local(path_buf));
                     added = true;
                 }
            }
        }

        if added {
            PersistenceManager::save_playlist(&player.playlist.tracks);
            app_handle.emit("playlist-updated", ()).unwrap();
        }
    }

    Ok(())
}

#[tauri::command]
async fn pick_and_add_folder(app_handle: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let folder_path = app_handle.dialog().file()
        .blocking_pick_folder();

    if let Some(folder) = folder_path {
        let folder_buf = match folder.into_path() {
            Ok(p) => p,
            Err(_) => return Err("Failed to get folder path".into()),
        };

        // Scan folder recursively and build tree structure
        let media_extensions = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mkv", "webm", "avi", "mov"];

        fn scan_directory_tree(dir: &std::path::Path, extensions: &[&str]) -> Option<PlaylistItem> {
            let folder_name = dir.file_name()?.to_string_lossy().to_string();
            let mut children = Vec::new();

            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut entries: Vec<_> = entries.flatten().collect();
                // Sort: folders first, then files
                entries.sort_by_key(|e| {
                    let path = e.path();
                    (!path.is_dir(), path.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
                });

                for entry in entries {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(subfolder) = scan_directory_tree(&path, extensions) {
                            children.push(subfolder);
                        }
                    } else if path.is_file() {
                        if let Some(ext) = path.extension() {
                            let ext_str = ext.to_string_lossy().to_lowercase();
                            if extensions.contains(&ext_str.as_str()) {
                                children.push(PlaylistItem::Track(Track::new_local(path)));
                            }
                        }
                    }
                }
            }

            if children.is_empty() {
                None
            } else {
                Some(PlaylistItem::Folder {
                    name: folder_name,
                    path: dir.to_path_buf(),
                    children,
                })
            }
        }

        if let Some(folder_item) = scan_directory_tree(&folder_buf, &media_extensions) {
            // Flatten the tree to add tracks to playlist
            fn flatten_items(item: &PlaylistItem, tracks: &mut Vec<Track>) {
                match item {
                    PlaylistItem::Track(track) => tracks.push(track.clone()),
                    PlaylistItem::Folder { children, .. } => {
                        for child in children {
                            flatten_items(child, tracks);
                        }
                    }
                }
            }

            let mut new_tracks = Vec::new();
            flatten_items(&folder_item, &mut new_tracks);

            if new_tracks.is_empty() {
                return Err("No media files found in the selected folder".into());
            }

            // Add tracks to playlist
            let mut player = state.0.lock().unwrap();
            let mut added = false;

            for track in new_tracks {
                let path = match &track.source {
                    TrackSource::Local(p) => p,
                    _ => continue,
                };

                let exists = player.playlist.tracks.iter().any(|t| {
                    match &t.source {
                        TrackSource::Local(p) => p.as_path() == path.as_path(),
                        _ => false
                    }
                });

                if !exists {
                    player.playlist.add_track(track);
                    added = true;
                }
            }

            if added {
                PersistenceManager::save_playlist(&player.playlist.tracks);
                app_handle.emit("playlist-updated", ()).unwrap();
            }
        } else {
            return Err("No media files found in the selected folder".into());
        }
    }

    Ok(())
}

#[tauri::command]
fn get_folder_tree(folder_path: String) -> Result<PlaylistItem, String> {
    let path = std::path::PathBuf::from(folder_path);
    let media_extensions = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mkv", "webm", "avi", "mov"];

    fn scan_directory_tree(dir: &std::path::Path, extensions: &[&str]) -> Option<PlaylistItem> {
        let folder_name = dir.file_name()?.to_string_lossy().to_string();
        let mut children = Vec::new();

        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| {
                let path = e.path();
                (!path.is_dir(), path.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
            });

            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(subfolder) = scan_directory_tree(&path, extensions) {
                        children.push(subfolder);
                    }
                } else if path.is_file() {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy().to_lowercase();
                        if extensions.contains(&ext_str.as_str()) {
                            children.push(PlaylistItem::Track(Track::new_local(path)));
                        }
                    }
                }
            }
        }

        if children.is_empty() {
            None
        } else {
            Some(PlaylistItem::Folder {
                name: folder_name,
                path: dir.to_path_buf(),
                children,
            })
        }
    }

    scan_directory_tree(&path, &media_extensions)
        .ok_or_else(|| "No media files found".to_string())
}

#[tauri::command]
async fn add_url_for_download(url: String, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // Clean URL (remove trailing characters like :)
    let url = url.trim().trim_end_matches(':').to_string();

    // Check for duplicates first (quick check)
    {
        let player = state.0.lock().unwrap();
        let exists = player.playlist.tracks.iter().any(|t| {
            match &t.source {
                TrackSource::Remote { url: u, .. } => u == &url,
                _ => false
            }
        });
        if exists {
            return Ok(()); // Already exists, skip
        }
    }

    // Emit loading state
    app_handle.emit("url-resolving", true).unwrap();

    // Resolve metadata in background thread to avoid blocking UI
    let url_clone = url.clone();
    let metadata_result = tauri::async_runtime::spawn_blocking(move || {
        OnlineResolver::resolve_metadata(&url_clone)
    }).await.map_err(|e| format!("Task failed: {}", e))?;

    // Emit loading done
    app_handle.emit("url-resolving", false).unwrap();

    let (title, duration, id, media_type) = match metadata_result {
        Ok(metadata) => {
            let media_type = metadata.get_media_type();
            (
                metadata.title,
                metadata.duration.map(|d| Duration::from_secs_f64(d)),
                metadata.id,
                media_type
            )
        },
        Err(e) => {
            return Err(format!("Failed to resolve URL: {}", e));
        }
    };

    // Add to playlist WITHOUT downloading - user must double-click to download
    let added = {
        let mut player = state.0.lock().unwrap();

        // Double check for duplicates (in case added while resolving)
        let exists = player.playlist.tracks.iter().any(|t| {
            match &t.source {
                TrackSource::Remote { url: u, .. } => u == &url,
                _ => false
            }
        });

        if !exists {
            let track = Track::new_remote(
                url,
                id,
                title,
                duration,
                media_type
            );
            player.playlist.add_track(track.clone());
            PersistenceManager::save_playlist(&player.playlist.tracks);
            true
        } else {
            false
        }
    };

    if added {
        app_handle.emit("playlist-updated", ()).unwrap();
    }
    Ok(())
}

#[tauri::command]
async fn download_and_play(index: usize, extra_subtitle_lang: Option<String>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // Get track info and check if already downloading
    let (url, id, title, media_type, already_cached, is_downloading) = {
        let player = state.0.lock().unwrap();
        if index >= player.playlist.tracks.len() {
            return Err("Index out of bounds".into());
        }

        let track = &player.playlist.tracks[index];
        match &track.source {
            TrackSource::Remote { url, id, title, media_type, cached_path, is_downloading, .. } => {
                // Check if already downloaded
                let cached = if let Some(path) = cached_path {
                    path.exists()
                } else {
                    false
                };
                (url.clone(), id.clone(), title.clone(), media_type.clone(), cached, *is_downloading)
            },
            TrackSource::Local(_) => {
                // Local file, play directly
                return Err("This is a local file, use play_track instead".into());
            }
        }
    }; // Lock dropped here

    // If already cached, play directly
    if already_cached {
        return play_track(index, state, app_handle).await;
    }

    // If already downloading, return error
    if is_downloading {
        return Err("Already downloading".into());
    }

    // Mark as downloading
    {
        let mut player = state.0.lock().unwrap();
        if let Some(track) = player.playlist.tracks.get_mut(index) {
            if let TrackSource::Remote { is_downloading, .. } = &mut track.source {
                *is_downloading = true;
            }
        }
    }
    app_handle.emit("playlist-updated", ()).unwrap();

    // Download to temp directory first
    let download_dir = get_download_dir();
    let cache_dir = get_cache_dir();

    // Ensure directories exist
    if !download_dir.exists() {
        std::fs::create_dir_all(&download_dir).map_err(|e| format!("Failed to create download dir: {}", e))?;
    }
    if !cache_dir.exists() {
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Failed to create cache dir: {}", e))?;
    }

    let app_handle_clone = app_handle.clone();
    let state_clone = state.0.clone();
    let index_clone = index;
    let url_clone = url.clone();
    let id_clone = id.clone();
    let download_dir_clone = download_dir.clone();

    // Download in a blocking task to not block the async runtime
    let download_result = tokio::task::spawn_blocking(move || {
        OnlineResolver::download_media(
            &url_clone,
            &id_clone,
            &download_dir_clone,
            media_type,
            extra_subtitle_lang.as_deref(),
            move |progress| {
                let _ = app_handle_clone.emit("download-progress", progress);
            }
        )
    }).await.map_err(|e| format!("Download task failed: {}", e))?;

    match download_result {
        Ok(temp_path) => {
            // Move file from download dir to cache dir
            let file_name = temp_path.file_name().unwrap_or_default();
            let final_path = cache_dir.join(file_name);

            // Remove existing file if exists
            if final_path.exists() {
                let _ = std::fs::remove_file(&final_path);
            }

            // Move file
            std::fs::rename(&temp_path, &final_path)
                .or_else(|_| {
                    // If rename fails (cross-device), try copy + delete
                    std::fs::copy(&temp_path, &final_path)?;
                    std::fs::remove_file(&temp_path)
                })
                .map_err(|e| format!("Failed to move file: {}", e))?;

            // Move subtitle files (if any) from download dir to cache dir
            if let Ok(entries) = std::fs::read_dir(&download_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name() {
                            let name_str = name.to_string_lossy();
                            // Check if it's a subtitle file for this video
                            if name_str.starts_with(&format!("{}.", id)) &&
                               (name_str.ends_with(".srt") ||
                                name_str.ends_with(".vtt") ||
                                name_str.ends_with(".ass") ||
                                name_str.ends_with(".ssa")) {
                                let sub_final_path = cache_dir.join(name);
                                let _ = std::fs::rename(&path, &sub_final_path)
                                    .or_else(|_| {
                                        std::fs::copy(&path, &sub_final_path)?;
                                        std::fs::remove_file(&path)
                                    });
                            }
                        }
                    }
                }
            }

            // Update track with cached path and new title (filename)
            {
                let mut player = state_clone.lock().unwrap();
                if let Some(track) = player.playlist.tracks.get_mut(index_clone) {
                    if let TrackSource::Remote { cached_path, title, is_downloading, .. } = &mut track.source {
                        println!("Updating track state: cached_path={:?}, is_downloading=false", final_path);
                        *cached_path = Some(final_path.clone());
                        *is_downloading = false;
                        // Update title to filename (without extension)
                        if let Some(stem) = final_path.file_stem() {
                            *title = stem.to_string_lossy().to_string();
                        }
                        println!("Track state updated successfully");
                    }
                } else {
                    println!("ERROR: Track at index {} not found!", index_clone);
                }
                PersistenceManager::save_playlist(&player.playlist.tracks);
            }

            println!("Emitting playlist-updated event");
            app_handle.emit("playlist-updated", ()).unwrap();

            // Now play the downloaded file
            play_track(index, state, app_handle).await
        },
        Err(e) => {
            // Reset downloading state on error
            {
                let mut player = state_clone.lock().unwrap();
                if let Some(track) = player.playlist.tracks.get_mut(index_clone) {
                    if let TrackSource::Remote { is_downloading, .. } = &mut track.source {
                        *is_downloading = false;
                    }
                }
            }
            app_handle.emit("playlist-updated", ()).unwrap();
            Err(format!("Download failed: {}", e))
        }
    }
}

#[tauri::command]
async fn on_playback_error(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    println!("Frontend playback failed");

    // Get current track info
    let (url_or_path, is_video) = {
        let player = state.0.lock().unwrap();

        if let Some(track) = player.playlist.current_track() {
            let url = match &track.source {
                TrackSource::Local(p) => p.to_string_lossy().to_string(),
                TrackSource::Remote { url, cached_path, .. } => {
                    if let Some(p) = cached_path {
                        p.to_string_lossy().to_string()
                    } else {
                        url.clone()
                    }
                }
            };

            let is_video = match &player.current_media_type {
                Some(MediaType::Video) => true,
                _ => false,
            };

            (url, is_video)
        } else {
            return Ok(());
        }
    };

    // Only use ffplay for audio files or as last resort for video
    // For video, the frontend video player should handle it
    if !is_video {
        // Try backend audio player for audio files
        if let Some(ffplay) = OnlineResolver::get_ffplay_path() {
            let play_target = if url_or_path.starts_with("http") {
                println!("Attempting to resolve stream URL for backend playback...");
                match OnlineResolver::get_stream_url(&url_or_path) {
                    Ok(u) => {
                        println!("Resolved stream URL: {}", u);
                        u
                    },
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
                // Add platform-specific referer based on original URL
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
                .arg("-window_title").arg("Drip Music Player")
                .spawn();

            match child_result {
                Ok(c) => {
                    let mut player = state.0.lock().unwrap();
                    // Kill existing process if any
                    if let Some(mut child) = player.video_process.take() {
                        let _ = child.kill();
                    }
                    player.video_process = Some(c);
                    player.is_playing = true;
                    player.playback_start = Some(Instant::now());
                },
                Err(e) => {
                    println!("Failed to start ffplay: {}", e);
                }
            }
        } else {
            let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let lib_path = current_dir.join("lib").join("ffplay.exe");
            println!("ffplay not found in lib or PATH");
            println!("Checked lib path: {}", lib_path.display());
            println!("Please download ffplay and place it in 'lib' folder or add to PATH");
        }
    } else {
        println!("Video playback error - frontend should handle video playback");
    }

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

#[tauri::command]
async fn show_track_context_menu(window: Window, index: usize, locale: String) -> Result<(), String> {
    let app_handle = window.app_handle().clone();

    let label = if locale == "zh" {
        "从播放列表移除"
    } else {
        "Remove from playlist"
    };

    let remove_item = MenuItem::with_id(
        &app_handle,
        format!("remove_{}", index),
        label,
        true,
        None::<&str>,
    ).map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        &app_handle,
        &[&remove_item],
    ).map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}

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
    ).map_err(|e| e.to_string())?;

    let clear_tree_item = MenuItem::with_id(
        &app_handle,
        "clear_tree",
        clear_tree_label,
        true,
        None::<&str>,
    ).map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        &app_handle,
        &[&clear_playlist_item, &clear_tree_item],
    ).map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn remove_track(index: usize, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    if index >= player.playlist.tracks.len() {
        return Err("Index out of bounds".into());
    }

    player.playlist.tracks.remove(index);

    // Adjust current index if needed
    if let Some(current) = player.playlist.current_index {
        if current == index {
            // Removed currently playing track
            player.playlist.current_index = None;
            player.is_playing = false;
            player.audio.stop();
        } else if current > index {
            // Adjust index if removed track was before current
            player.playlist.current_index = Some(current - 1);
        }
    }

    PersistenceManager::save_playlist(&player.playlist.tracks);
    app_handle.emit("playlist-updated", ()).unwrap();
    Ok(())
}

#[tauri::command]
async fn clear_playlist(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    player.playlist.tracks.clear();
    player.playlist.current_index = None;
    player.is_playing = false;
    player.audio.stop();

    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }

    PersistenceManager::save_playlist(&player.playlist.tracks);
    app_handle.emit("playlist-updated", ()).unwrap();
    Ok(())
}

#[tauri::command]
fn check_dependencies() -> Result<serde_json::Value, String> {
    use crate::services::online_resolver::OnlineResolver;
    use serde_json::json;

    let (yt_dlp_cmd, ffmpeg_dir) = OnlineResolver::get_tools_paths();
    let ffplay_path = OnlineResolver::get_ffplay_path();

    // Check yt-dlp
    let yt_dlp_available = hidden_command(&yt_dlp_cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // Check ffmpeg
    let ffmpeg_available = if let Some(dir) = ffmpeg_dir {
        let ffmpeg_exe = std::path::PathBuf::from(&dir).join("ffmpeg.exe");
        ffmpeg_exe.exists()
    } else {
        hidden_command("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };

    // Check ffplay
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
            "purpose": "Fallback video player"
        }
    }))
}

#[tauri::command]
fn get_video_url(path: String) -> Result<String, String> {
    let path_buf = std::path::PathBuf::from(&path);

    if !path_buf.exists() {
        return Err(format!("File not found: {}", path));
    }

    // Return the path as-is, convertFileSrc will handle it on the frontend
    Ok(path)
}

#[tauri::command]
async fn play_online_video(window: Window, url: String) -> Result<(), String> {
    let platform = VideoPlatform::from_url(&url);
    println!("Resolving {} video URL: {}", platform.display_name(), url);

    let video_url = OnlineResolver::get_stream_url(&url)
        .map_err(|e| format!("Failed to get video: {}", e))?;

    println!("Resolved video URL: {}", video_url);

    // Emit to frontend
    window.emit("online_video_url", video_url)
        .map_err(|e| format!("Failed to emit event: {}", e))?;

    Ok(())
}

// Keep old name for backward compatibility
#[tauri::command]
async fn play_bilibili_video(window: Window, url: String) -> Result<(), String> {
    play_online_video(window, url).await
}

#[tauri::command]
async fn play_with_mpv(path: String, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    println!("Playing with MPV: {}", path);

    // Get MPV path
    let mpv_path = OnlineResolver::get_mpv_path()
        .ok_or_else(|| "MPV not found. Please install MPV or place mpv.exe in the lib folder.".to_string())?;

    println!("Using MPV: {}", mpv_path);

    // Kill existing video process
    {
        let mut player = state.0.lock().unwrap();
        if let Some(mut child) = player.video_process.take() {
            let _ = child.kill();
        }
    }

    // Spawn MPV process
    let child = Command::new(&mpv_path)
        .arg(&path)
        .arg("--force-window=yes")
        .arg("--title=Drip Player")
        .arg("--osd-level=1")
        .spawn()
        .map_err(|e| format!("Failed to start MPV: {}", e))?;

    // Store process and update state
    {
        let mut player = state.0.lock().unwrap();
        player.video_process = Some(child);
        player.is_playing = true;
        player.playback_start = Some(Instant::now());
    }

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

#[tauri::command]
fn check_mpv_available() -> bool {
    OnlineResolver::get_mpv_path().is_some()
}

/// Open the login page for a platform in the default browser
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

    // Open in default browser
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

/// Check if an error indicates login is required and return login info
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

/// Try to add URL using OAuth2 authentication (for YouTube)
/// This will trigger browser-based OAuth flow
#[tauri::command]
async fn add_url_with_oauth(url: String, window: Window) -> Result<(), String> {
    println!("Attempting to add URL with OAuth2: {}", url);

    // Emit resolving state
    window.emit("url-resolving", true).ok();

    let url_clone = url.clone();
    let result = tokio::task::spawn_blocking(move || {
        OnlineResolver::resolve_metadata_with_oauth(&url_clone)
    }).await.map_err(|e| format!("Task failed: {}", e))?;

    window.emit("url-resolving", false).ok();

    match result {
        Ok(metadata) => {
            println!("OAuth2 resolved: {} ({})", metadata.title, metadata.id);
            // Now add to playlist using the regular flow
            // We need to call add_url_for_download logic here
            // For now, emit success and let frontend retry with normal flow
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

#[tauri::command]
fn scan_subtitles(video_path: String) -> Vec<SubtitleInfo> {
    let video_path = std::path::Path::new(&video_path);
    let mut subtitles = Vec::new();

    // Get the directory and file stem of the video
    let parent = match video_path.parent() {
        Some(p) => p,
        None => return subtitles,
    };

    let stem = match video_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return subtitles,
    };

    // Scan for subtitle files with same stem
    let subtitle_extensions = ["srt", "vtt", "ass", "ssa"];

    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if subtitle_extensions.contains(&ext_str.as_str()) {
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                        // Check if subtitle file matches video stem
                        // Patterns: stem.lang.ext or stem.ext
                        if file_name.starts_with(&format!("{}.", stem)) {
                            // Extract language from filename
                            let name_without_ext = path.file_stem().unwrap_or_default().to_string_lossy();
                            let lang = if name_without_ext.len() > stem.len() + 1 {
                                // Has language code: stem.lang
                                name_without_ext[stem.len() + 1..].to_string()
                            } else {
                                // No language code, use extension as identifier
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

    // Sort by language
    subtitles.sort_by(|a, b| a.lang.cmp(&b.lang));
    subtitles
}

fn main() {
    let player = Arc::new(Mutex::new(MusicPlayer::new()));

    // Start proxy server
    tauri::async_runtime::spawn(async {
        services::stream_server::start_server(10001).await;
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(player))
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>().inner().clone();
            let state_for_menu = state.clone();

            // Menu event handler
            app.on_menu_event(move |app, event| {
                let event_id = event.id().as_ref();

                if event_id.starts_with("remove_") {
                    if let Some(index_str) = event_id.strip_prefix("remove_") {
                        if let Ok(index) = index_str.parse::<usize>() {
                            let state_clone = state_for_menu.clone();
                            let app_clone = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let mut player = state_clone.0.lock().unwrap();
                                if index < player.playlist.tracks.len() {
                                    player.playlist.tracks.remove(index);
                                    if let Some(current) = player.playlist.current_index {
                                        if current == index {
                                            player.playlist.current_index = None;
                                            player.is_playing = false;
                                            player.audio.stop();
                                        } else if current > index {
                                            player.playlist.current_index = Some(current - 1);
                                        }
                                    }
                                    PersistenceManager::save_playlist(&player.playlist.tracks);
                                    let _ = app_clone.emit("playlist-updated", ());
                                }
                            });
                        }
                    }
                } else if event_id == "clear_playlist" {
                    let state_clone = state_for_menu.clone();
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut player = state_clone.0.lock().unwrap();
                        player.playlist.tracks.clear();
                        player.playlist.current_index = None;
                        player.is_playing = false;
                        player.audio.stop();
                        if let Some(mut child) = player.video_process.take() {
                            let _ = child.kill();
                        }
                        PersistenceManager::save_playlist(&player.playlist.tracks);
                        let _ = app_clone.emit("playlist-updated", ());
                    });
                } else if event_id == "clear_tree" {
                    let _ = app.emit("clear-folder-tree", ());
                }
            });

            // Move cache scanning to background thread to avoid blocking startup
            tauri::async_runtime::spawn_blocking(move || {
                let cached_tracks = PersistenceManager::scan_cache_for_tracks();
                if !cached_tracks.is_empty() {
                    let mut player = state.0.lock().unwrap();
                    let mut added = false;
                    for track in cached_tracks {
                        let exists = player.playlist.tracks.iter().any(|t| {
                            match (&t.source, &track.source) {
                                (TrackSource::Local(p1), TrackSource::Local(p2)) => p1 == p2,
                                (TrackSource::Remote { cached_path: Some(p1), .. }, TrackSource::Local(p2)) => p1 == p2,
                                 _ => false
                            }
                        });

                        if !exists {
                            player.playlist.add_track(track);
                            added = true;
                        }
                    }

                    if added {
                        let _ = handle.emit("playlist-updated", ());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_playlist,
            play_track,
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
