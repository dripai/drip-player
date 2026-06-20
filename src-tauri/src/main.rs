#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

mod models;
mod services;
mod utils;
mod handlers;

use models::player_state::{MusicPlayer, PlayerState};
use models::playlist::{MediaType, TrackSource, LibraryItem, PlaylistEntry};
use services::media_capabilities;
use services::media_probe::{self, MediaInfo};
use services::online_resolver::{OnlineResolver, VideoPlatform};
use services::persistence::PersistenceManager;
use services::playback_plan::{self, PlaybackPlan};
use services::toolchain;
use std::sync::{Arc, Mutex};
use tauri::{State, AppHandle, Window, Emitter, Manager, menu::{Menu, MenuItem, ContextMenu, CheckMenuItem}, tray::{TrayIconBuilder, TrayIconEvent, MouseButton}};
use std::time::{Duration, Instant};
use std::process::Command;
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
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
        .join("cache")
}

/// 获取下载临时目录
fn get_download_dir() -> std::path::PathBuf {
    get_cache_dir().join("downloading")
}

impl Deref for AppState {
    type Target = Arc<Mutex<MusicPlayer>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 比较两个路径是否相同（在 Windows 上忽略大小写）
fn paths_match(p1: &Path, p2: &Path) -> bool {
    if p1 == p2 {
        return true;
    }
    
    #[cfg(windows)]
    {
        return p1.to_string_lossy().to_lowercase() == p2.to_string_lossy().to_lowercase();
    }

    #[cfg(not(windows))]
    false
}

fn media_type_for_library_path(path: &Path) -> MediaType {
    media_probe::probe(path)
        .map(|info| info.media_type)
        .unwrap_or_else(|| media_capabilities::media_type_from_path(path))
}

/// 获取播放列表（返回 PlaylistEntry 列表）
#[tauri::command]
fn get_playlist(state: State<AppState>) -> Vec<PlaylistEntry> {
    let player = state.0.lock().unwrap();
    // 新流程：返回基于 state 存储的 PlaylistEntry 列表
    player.playlist_entries.clone()
}

/// 获取库树
#[tauri::command]
fn get_library_tree(state: State<AppState>) -> Vec<LibraryItem> {
    let player = state.0.lock().unwrap();
    player.library.clone()
}

/// 向库中添加项（只支持远程或本地单项）
#[tauri::command]
fn add_library_item(item: LibraryItem, state: State<AppState>) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    player.library.push(item);
    Ok(())
}

/// 从库中移除项（按 id 或 path 匹配）
#[tauri::command]
fn remove_library_item(id_or_path: String, state: State<AppState>) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    player.library.retain(|it| match it {
        LibraryItem::Track { id, .. } => id != &id_or_path,
        LibraryItem::Folder { path, .. } => path.to_string_lossy() != id_or_path,
    });
    Ok(())
}

/// 将库项加入播放列表
#[tauri::command]
fn add_to_playlist(item_id: String, state: State<AppState>) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id, added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
    player.playlist_entries.push(entry);
    Ok(())
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
                if player.progress > 1.0 { player.progress = 1.0; }
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

    let current_item = if let Some(index) = player.current_playlist_index {
        player.playlist_entries.get(index).and_then(|entry| {
            player.library.iter().find_map(|it| match it {
                LibraryItem::Track { id, .. } if id == &entry.item_id => Some(it.clone()),
                _ => None,
            })
        })
    } else {
        player.temporary_item.clone()
    };
    PlayerState {
        is_playing: player.is_playing,
        progress: player.progress,
        duration: player.duration.as_secs_f64(),
        current_index: player.current_playlist_index,
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
                    Ok(Ok(_)) => {},
                    Ok(Err(e)) => {
                        println!("Playback error: {}", e);
                        let mut player = player_handle.lock().unwrap();
                        player.is_playing = false;
                        player.playback_start = None;
                        drop(player);
                        let _ = app_handle_clone.emit("playback-error", e);
                        let _ = app_handle_clone.emit("player-state-changed", ());
                    },
                    Err(_) => {}
                }
            });
        }
        PlaybackPlan::ExternalVideo { path } => {
            let mpv_path = OnlineResolver::get_mpv_path()
                .ok_or_else(|| format!("MPV not found in {}", toolchain::diagnostic_lib_dir().display()))?;
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

/// 播放指定索引的曲目（索引指向 `playlist_entries`）
#[tauri::command]
async fn play_track(index: usize, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
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
    player.temporary_item = None; // 清除临时项

    if index >= player.playlist_entries.len() {
        return Err("Index out of bounds".into());
    }

    // 查找对应的库项
    let entry = player.playlist_entries[index].clone();
    let lib_idx = player.library.iter().position(|it| match it {
        LibraryItem::Track { id, .. } => id == &entry.item_id,
        _ => false,
    }).ok_or_else(|| "Library item not found".to_string())?;

    let item = player.library[lib_idx].clone();
    let plan = playback_plan::plan_for_item(&item)?;

    player.current_playlist_index = Some(index);
    apply_playback_plan(plan, &mut player, state.0.clone(), app_handle.clone())?;

    app_handle.emit("player-state-changed", ()).unwrap();
    Ok(())
}

/// 直接播放曲目（不加入播放列表）
#[tauri::command]
async fn play_track_directly(item: LibraryItem, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
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

    // 设置临时项并清除当前播放索引
    player.current_playlist_index = None;
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
async fn add_local_files(paths: Vec<String>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();
    let mut added = false;
    
    for path_str in paths {
        let path = std::path::PathBuf::from(path_str);
        if path.exists() {
            // 检查库中是否已有此本地项
            let exists = player.library.iter().any(|it| match it {
                LibraryItem::Track { source, .. } => match source {
                    crate::models::playlist::LibrarySource::Local { path: p } => paths_match(p, &path),
                    _ => false,
                },
                _ => false,
            });

            if !exists {
                let id = path.to_string_lossy().to_string();
                let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let item = LibraryItem::Track { id: id.clone(), title: title.clone(), media_type: media_type_for_library_path(&path), source: crate::models::playlist::LibrarySource::Local { path: path.clone() }, parent: path.parent().map(|p| p.to_path_buf()) };
                player.library.push(item);
                // 同时加入播放列表
                let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id: id, added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
                player.playlist_entries.push(entry);
                added = true;
            }
        }
    }

    if added {
        PersistenceManager::save_library(&player.library);
        PersistenceManager::save_playlist_entries(&player.playlist_entries);
        app_handle.emit("playlist-updated", ()).unwrap();
    }
    Ok(())
}

/// 打开文件选择对话框并添加本地文件
#[tauri::command]
async fn pick_and_add_local_files(app_handle: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let file_paths = app_handle.dialog().file()
        .add_filter("Media Files", media_capabilities::MEDIA_EXTENSIONS)
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
                    let exists = player.library.iter().any(|it| match it {
                        LibraryItem::Track { source, .. } => match source {
                            crate::models::playlist::LibrarySource::Local { path: p } => paths_match(p, &path_buf),
                            _ => false,
                        },
                        _ => false,
                    });

                    if !exists {
                        let id = path_buf.to_string_lossy().to_string();
                        let title = path_buf.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        let item = LibraryItem::Track { id: id.clone(), title: title.clone(), media_type: media_type_for_library_path(&path_buf), source: crate::models::playlist::LibrarySource::Local { path: path_buf.clone() }, parent: path_buf.parent().map(|p| p.to_path_buf()) };
                        player.library.push(item);
                        let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id: id, added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
                        player.playlist_entries.push(entry);
                        added = true;
                    }
                }
            }

            if added {
                PersistenceManager::save_library(&player.library);
                PersistenceManager::save_playlist_entries(&player.playlist_entries);
                app_handle.emit("playlist-updated", ()).unwrap();
            }
    }

    Ok(())
}

/// 打开文件夹选择对话框并递归添加媒体文件
#[tauri::command]
async fn pick_and_add_folder(app_handle: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let folder_path = app_handle.dialog().file()
        .blocking_pick_folder();

    if let Some(folder) = folder_path {
        let folder_buf = match folder.into_path() {
            Ok(p) => p,
            Err(_) => return Err("Failed to get folder path".into()),
        };

        // 递归扫描文件夹并构建树形结构
        fn scan_directory_tree(dir: &std::path::Path) -> Option<LibraryItem> {
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
                        if let Some(subfolder) = scan_directory_tree(&path) {
                            children.push(subfolder);
                        }
                    } else if path.is_file() {
                        if media_capabilities::is_supported_media_path(&path) {
                            let id = path.to_string_lossy().to_string();
                            let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                            children.push(LibraryItem::Track { id: id.clone(), title, media_type: media_type_for_library_path(&path), source: crate::models::playlist::LibrarySource::Local { path: path.clone() }, parent: path.parent().map(|p| p.to_path_buf()) });
                        }
                    }
                }
            }

            if children.is_empty() {
                None
            } else {
                Some(LibraryItem::Folder { name: folder_name, path: dir.to_path_buf(), children })
            }
        }

        if let Some(folder_item) = scan_directory_tree(&folder_buf) {
            // 扁平化树以添加曲目到库并生成播放列表项
            fn flatten_items(item: &LibraryItem, tracks: &mut Vec<LibraryItem>) {
                match item {
                    LibraryItem::Track { .. } => tracks.push(item.clone()),
                    LibraryItem::Folder { children, .. } => {
                        for child in children {
                            flatten_items(child, tracks);
                        }
                    }
                }
            }

            let mut new_items = Vec::new();
            flatten_items(&folder_item, &mut new_items);

            if new_items.is_empty() {
                return Err("No media files found in the selected folder".into());
            }

            // 添加到库并加入播放队列
            let mut player = state.0.lock().unwrap();
            let mut added = false;

            for item in new_items {
                // 仅处理 Track 项
                if let LibraryItem::Track { id, .. } = &item {
                    let exists = player.library.iter().any(|it| match it {
                        LibraryItem::Track { id: existing_id, .. } => existing_id == id,
                        _ => false,
                    });

                    if !exists {
                        player.library.push(item.clone());
                        let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id: id.clone(), added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
                        player.playlist_entries.push(entry);
                        added = true;
                    }
                }
            }

            if added {
                PersistenceManager::save_library(&player.library);
                PersistenceManager::save_playlist_entries(&player.playlist_entries);
                app_handle.emit("playlist-updated", ()).unwrap();
            }
        } else {
            return Err("No media files found in the selected folder".into());
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
                (!path.is_dir(), path.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
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
                        let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        children.push(LibraryItem::Track { id, title, media_type: media_type_for_library_path(&path), source: crate::models::playlist::LibrarySource::Local { path: path.clone() }, parent: path.parent().map(|p| p.to_path_buf()) });
                    }
                }
            }
        }

        if children.is_empty() {
            None
        } else {
            Some(LibraryItem::Folder { name: folder_name, path: dir.to_path_buf(), children })
        }
    }

    scan_directory_tree(&path)
        .ok_or_else(|| "No media files found".to_string())
}

/// 添加 URL 进行下载解析
#[tauri::command]
async fn add_url_for_download(url: String, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // 清理 URL (移除尾部的字符，如 :)
    let url = url.trim().trim_end_matches(':').to_string();

    // 首先检查库中是否已存在相同 URL
    {
        let player = state.0.lock().unwrap();
        let exists = player.library.iter().any(|it| match it {
            LibraryItem::Track { source, .. } => match source {
                crate::models::playlist::LibrarySource::Remote { url: u, .. } => u == &url,
                _ => false,
            },
            _ => false,
        });
        if exists {
            return Ok(());
        }
    }

    // 发送加载状态
    app_handle.emit("url-resolving", true).unwrap();

    // 在后台线程中解析元数据以避免阻塞 UI
    let url_clone = url.clone();
    let metadata_result = tauri::async_runtime::spawn_blocking(move || {
        OnlineResolver::resolve_metadata(&url_clone)
    }).await.map_err(|e| format!("Task failed: {}", e))?;

    // 发送加载完成
    app_handle.emit("url-resolving", false).unwrap();

    let (title, _duration, id, media_type) = match metadata_result {
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

    // 添加到库并生成播放列表项（不立即下载）
    let added = {
        let mut player = state.0.lock().unwrap();
        let item_id = id.clone();
        let lib_item = LibraryItem::Track { id: item_id.clone(), title: title.clone(), media_type: media_type.clone(), source: crate::models::playlist::LibrarySource::Remote { url: url.clone(), id: id.clone(), cached_path: None, media_type: media_type.clone(), download_status: crate::models::playlist::DownloadStatus::NotDownloaded }, parent: None };
        player.library.push(lib_item);
        let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id: item_id.clone(), added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
        player.playlist_entries.push(entry);
        PersistenceManager::save_library(&player.library);
        PersistenceManager::save_playlist_entries(&player.playlist_entries);
        true
    };

    if added {
        app_handle.emit("playlist-updated", ()).unwrap();
    }
    Ok(())
}

/// 下载并播放指定索引的曲目
#[tauri::command]
async fn download_and_play(index: usize, extra_subtitle_lang: Option<String>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // 获取播放列表项对应的库项并检查状态
    let (url, id, title, media_type, already_cached) = {
        let player = state.0.lock().unwrap();
        if index >= player.playlist_entries.len() {
            return Err("Index out of bounds".into());
        }
        let entry = &player.playlist_entries[index];
        // 在库中查找
        let lib_item = player.library.iter().find(|it| match it {
            LibraryItem::Track { id: item_id, .. } => item_id == &entry.item_id,
            _ => false,
        }).ok_or_else(|| "Library item not found".to_string())?;

        match lib_item {
            LibraryItem::Track { title, media_type, source, .. } => match source {
                crate::models::playlist::LibrarySource::Remote { url, id, cached_path, .. } => {
                    let cached = cached_path.as_ref().map(|p| p.exists()).unwrap_or(false);
                    (url.clone(), id.clone(), title.clone(), media_type.clone(), cached)
                }
                crate::models::playlist::LibrarySource::Local { .. } => {
                    return Err("This is a local file, use play_track instead".into());
                }
            },
            _ => return Err("Selected playlist entry is not a track".into()),
        }
    };

    // 如果已缓存，直接播放
    if already_cached {
        return play_track(index, state, app_handle).await;
    }


    // 标记库项为正在下载
    {
        let mut player = state.0.lock().unwrap();
        let entry = player.playlist_entries.get(index).cloned().ok_or_else(|| "Index out of bounds".to_string())?;
        if let Some(it) = player.library.iter_mut().find(|it| match it { LibraryItem::Track { id, .. } => id == &entry.item_id, _ => false }) {
            if let LibraryItem::Track { source, .. } = it {
                if let crate::models::playlist::LibrarySource::Remote { download_status, .. } = source {
                    *download_status = crate::models::playlist::DownloadStatus::Downloading;
                }
            }
        }
        PersistenceManager::save_library(&player.library);
    }
    app_handle.emit("playlist-updated", ()).unwrap();

    // 首先下载到临时目录
    let download_dir = get_download_dir();
    let cache_dir = get_cache_dir();

    // 确保目录存在
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
    let title_for_download = title.clone();
    let title_for_download_clone = title_for_download.clone();
    let download_dir_clone = download_dir.clone();

    // 在阻塞任务中下载，以免阻塞异步运行时
    let download_result = tokio::task::spawn_blocking(move || {
        OnlineResolver::download_media(
            &url_clone,
            &id_clone,
            &title_for_download_clone,
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
            // 将文件从下载目录移动到缓存目录
            let file_name = temp_path.file_name().unwrap_or_default();
            let final_path = cache_dir.join(file_name);

            // 如果存在同名文件，则在文件名末尾添加流水号直到不冲突
            let safe_title = crate::services::online_resolver::OnlineResolver::sanitize_filename(&title_for_download);
            let mut candidate = final_path.clone();
            if candidate.exists() {
                let mut n = 1;
                let ext = final_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                while candidate.exists() {
                    let numbered_name = if ext.is_empty() {
                        format!("{} - {}", safe_title, n)
                    } else {
                        format!("{} - {}.{}", safe_title, n, ext)
                    };
                    candidate = cache_dir.join(numbered_name);
                    n += 1;
                }
            }

            // 移动/复制到确定的不冲突文件名
            std::fs::rename(&temp_path, &candidate)
                .or_else(|_| {
                    std::fs::copy(&temp_path, &candidate)?;
                    std::fs::remove_file(&temp_path)
                })
                .map_err(|e| format!("Failed to move file: {}", e))?;

            let final_path = candidate;

            // 将字幕文件（如果有）从下载目录移动到缓存目录
            let safe_title_for_subs = crate::services::online_resolver::OnlineResolver::sanitize_filename(&title_for_download);
            if let Ok(entries) = std::fs::read_dir(&download_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name() {
                            let name_str = name.to_string_lossy();
                            // 检查是否为该视频的字幕文件
                            let stem = name_str.split('.').next().unwrap_or("");
                            let title_match = stem == safe_title_for_subs || stem.starts_with(&format!("{} - ", safe_title_for_subs)) || stem.contains(&safe_title_for_subs);
                            if (title_match || name_str.contains(&id)) &&
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

            // 更新库项的缓存路径和下载状态
            {
                let mut player = state_clone.lock().unwrap();
                let entry = player.playlist_entries.get(index_clone).cloned();
                if let Some(entry) = entry {
                    if let Some(it) = player.library.iter_mut().find(|it| match it { LibraryItem::Track { id, .. } => id == &entry.item_id, _ => false }) {
                        if let LibraryItem::Track { source, .. } = it {
                            if let crate::models::playlist::LibrarySource::Remote { cached_path, download_status, .. } = source {
                                *cached_path = Some(final_path.clone());
                                *download_status = crate::models::playlist::DownloadStatus::Downloaded;
                            }
                        }
                    }
                }
                PersistenceManager::save_library(&player.library);
            }

            println!("Emitting playlist-updated event");
            app_handle.emit("playlist-updated", ()).unwrap();

            // 现在播放已下载的文件
            play_track(index, state, app_handle).await
        },
        Err(e) => {
            // 出错时重置库项下载状态
            {
                let mut player = state_clone.lock().unwrap();
                let entry = player.playlist_entries.get(index_clone).cloned();
                if let Some(entry) = entry {
                    if let Some(it) = player.library.iter_mut().find(|it| match it { LibraryItem::Track { id, .. } => id == &entry.item_id, _ => false }) {
                        if let LibraryItem::Track { source, .. } = it {
                            if let crate::models::playlist::LibrarySource::Remote { download_status, .. } = source {
                                *download_status = crate::models::playlist::DownloadStatus::NotDownloaded;
                            }
                        }
                    }
                }
                PersistenceManager::save_library(&player.library);
            }
            app_handle.emit("playlist-updated", ()).unwrap();
            Err(format!("Download failed: {}", e))
        }
    }
}

/// 播放出错时的处理 (尝试使用后端播放器)
#[tauri::command]
async fn on_playback_error(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    println!("Frontend playback failed");

    // 获取当前曲目信息（基于 playlist_entries + library 或 temporary_item）
    let (url_or_path, is_video) = {
        let player = state.0.lock().unwrap();

        // 优先使用正在播放的 playlist entry
        if let Some(idx) = player.current_playlist_index {
            if let Some(entry) = player.playlist_entries.get(idx) {
                if let Some(lib_item) = player.library.iter().find(|it| match it { LibraryItem::Track { id, .. } => id == &entry.item_id, _ => false }) {
                    if let LibraryItem::Track { source, media_type, .. } = lib_item {
                        let url = match source {
                            crate::models::playlist::LibrarySource::Local { path } => path.to_string_lossy().to_string(),
                            crate::models::playlist::LibrarySource::Remote { url, cached_path, .. } => {
                                if let Some(p) = cached_path { p.to_string_lossy().to_string() } else { url.clone() }
                            }
                        };
                        let is_video = match media_type { MediaType::Video => true, _ => false };
                        (url, is_video)
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        } else if let Some(item) = &player.temporary_item {
            // 使用临时项
            if let LibraryItem::Track { source, media_type, .. } = item {
                let url = match source {
                    crate::models::playlist::LibrarySource::Local { path } => path.to_string_lossy().to_string(),
                    crate::models::playlist::LibrarySource::Remote { url, cached_path, .. } => {
                        if let Some(p) = cached_path { p.to_string_lossy().to_string() } else { url.clone() }
                    }
                };
                let is_video = match media_type { MediaType::Video => true, _ => false };
                (url, is_video)
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
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
                .arg("-window_title").arg("Drip Music Player")
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
                },
                Err(e) => {
                    println!("Failed to start ffplay: {}", e);
                }
            }
        } else {
            let lib_path = toolchain::diagnostic_lib_dir().join(toolchain::executable_name("ffplay"));
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

/// 移除指定索引的曲目
#[tauri::command]
async fn remove_track(index: usize, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    if index >= player.playlist_entries.len() {
        return Err("Index out of bounds".into());
    }

    // 移除播放列表项
    player.playlist_entries.remove(index);

    // 如果需要，调整当前索引
    if let Some(current) = player.current_playlist_index {
        if current == index {
            player.current_playlist_index = None;
            player.is_playing = false;
            player.audio.stop();
        } else if current > index {
            player.current_playlist_index = Some(current - 1);
        }
    }

    PersistenceManager::save_playlist_entries(&player.playlist_entries);
    app_handle.emit("playlist-updated", ()).unwrap();
    Ok(())
}

/// 清空播放列表
#[tauri::command]
async fn clear_playlist(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let mut player = state.0.lock().unwrap();

    player.playlist_entries.clear();
    player.current_playlist_index = None;
    player.is_playing = false;
    player.audio.stop();

    if let Some(mut child) = player.video_process.take() {
        let _ = child.kill();
    }

    PersistenceManager::save_playlist_entries(&player.playlist_entries);
    app_handle.emit("playlist-updated", ()).unwrap();
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

    let video_url = OnlineResolver::get_stream_url(&url)
        .map_err(|e| format!("Failed to get video: {}", e))?;

    println!("Resolved video URL: {}", video_url);

    // 发送到前端
    window.emit("online_video_url", video_url)
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
async fn play_with_mpv(path: String, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    println!("Playing with MPV: {}", path);

    // 获取 MPV 路径
    let mpv_path = OnlineResolver::get_mpv_path()
        .ok_or_else(|| format!("MPV not found in {}", toolchain::diagnostic_lib_dir().display()))?;

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
    }).await.map_err(|e| format!("Task failed: {}", e))?;

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
                            let name_without_ext = path.file_stem().unwrap_or_default().to_string_lossy();
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
    let player = Arc::new(Mutex::new(MusicPlayer::new()));

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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let player = state.0.lock().unwrap();
                if player.minimize_to_tray {
                    api.prevent_close();
                    window.hide().unwrap();
                }
            }
        })
        .setup(|app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                services::toolchain::set_resource_dir(resource_dir);
            }

            let handle = app.handle().clone();
            let state = app.state::<AppState>().inner().clone();
            let state_for_menu = state.clone();

            // 系统托盘配置
            let initial_minimize_to_tray = state.0.lock().unwrap().minimize_to_tray;
            let quit_i = MenuItem::with_id(app, "tray_quit", "退出", true, None::<&str>)?;
            let restore_i = MenuItem::with_id(app, "tray_restore", "恢复窗口", true, None::<&str>)?;
            let minimize_on_close_i = CheckMenuItem::with_id(app, "tray_minimize_on_close", "关闭时最小化", true, initial_minimize_to_tray, None::<&str>)?;
            
            let tray_menu = Menu::with_items(app, &[&restore_i, &minimize_on_close_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
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
                    let mut player = state_for_menu.0.lock().unwrap();
                    player.minimize_to_tray = !player.minimize_to_tray;
                    
                    // 保存设置
                    let settings = services::persistence::AppSettings {
                        minimize_to_tray: player.minimize_to_tray,
                    };
                    PersistenceManager::save_settings(&settings);
                }
                if event_id.starts_with("remove_") {
                    if let Some(index_str) = event_id.strip_prefix("remove_") {
                        if let Ok(index) = index_str.parse::<usize>() {
                            let state_clone = state_for_menu.clone();
                            let app_clone = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let mut player = state_clone.0.lock().unwrap();
                                if index < player.playlist_entries.len() {
                                    player.playlist_entries.remove(index);
                                    if let Some(current) = player.current_playlist_index {
                                        if current == index {
                                            player.current_playlist_index = None;
                                            player.is_playing = false;
                                            player.audio.stop();
                                        } else if current > index {
                                            player.current_playlist_index = Some(current - 1);
                                        }
                                    }
                                    PersistenceManager::save_playlist_entries(&player.playlist_entries);
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
                        player.playlist_entries.clear();
                        player.current_playlist_index = None;
                        player.is_playing = false;
                        player.audio.stop();
                        if let Some(mut child) = player.video_process.take() {
                            let _ = child.kill();
                        }
                        PersistenceManager::save_playlist_entries(&player.playlist_entries);
                        let _ = app_clone.emit("playlist-updated", ());
                    });
                } else if event_id == "clear_tree" {
                    let _ = app.emit("clear-folder-tree", ());
                }
            });

            // 将缓存扫描移动到后台线程以避免阻塞启动
            tauri::async_runtime::spawn_blocking(move || {
                let cached_tracks = PersistenceManager::scan_cache_for_tracks();
                if !cached_tracks.is_empty() {
                    let mut player = state.0.lock().unwrap();
                    let mut added = false;
                    for track in cached_tracks {
                        // Determine path of track
                        let path_opt = match &track.source {
                            TrackSource::Local(p) => Some(p.clone()),
                            TrackSource::Remote { cached_path: Some(p), .. } => Some(p.clone()),
                            _ => None,
                        };

                        if let Some(path) = path_opt {
                            let exists = player.library.iter().any(|it| match it {
                                LibraryItem::Track { source, .. } => match source {
                                    crate::models::playlist::LibrarySource::Local { path: p } => paths_match(p, &path),
                                    crate::models::playlist::LibrarySource::Remote { cached_path: Some(p), .. } => paths_match(p, &path),
                                    _ => false,
                                },
                                _ => false,
                            });

                            if !exists {
                                // Add to library
                                let id = path.to_string_lossy().to_string();
                                let title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                                let lib_item = LibraryItem::Track { id: id.clone(), title: title.clone(), media_type: media_type_for_library_path(&path), source: crate::models::playlist::LibrarySource::Local { path: path.clone() }, parent: path.parent().map(|p| p.to_path_buf()) };
                                player.library.push(lib_item);
                                let entry = PlaylistEntry { id: uuid::Uuid::new_v4().to_string(), item_id: id.clone(), added_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() };
                                player.playlist_entries.push(entry);
                                added = true;
                            }
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
            get_library_tree,
            add_library_item,
            remove_library_item,
            add_to_playlist,
            get_playback_plan,
            probe_media,
            play_track,
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
