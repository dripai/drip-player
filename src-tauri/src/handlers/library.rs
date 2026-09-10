use crate::app_state::{lock, AppState};
use crate::application;
use crate::models::library::LibraryItem;
use crate::models::media::{AssetKind, MediaAsset};
use crate::models::playlist::PlaylistSnapshot;
use crate::services::media_capabilities;
use crate::services::playlist_files::{self, PlaylistFile};
use std::path::Path;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub fn get_playlist(state: State<AppState>) -> Result<PlaylistSnapshot, String> {
    application::playlist_snapshot(&state)
}

#[tauri::command]
pub async fn get_playlist_file(
    state: State<'_, AppState>,
    item_id: String,
) -> Result<PlaylistFile, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || playlist_files::get_file(&state, &item_id))
        .await
        .map_err(|error| error.to_string())?
}

async fn change_file(
    app: AppHandle,
    state: AppState,
    target: PlaylistFile,
    name: Option<String>,
) -> Result<(), String> {
    let result = tauri::async_runtime::spawn_blocking(move || match name {
        Some(name) => playlist_files::rename(&state, &target, &name),
        None => playlist_files::delete(&state, &target),
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    let mut errors: Vec<String> = result.err().into_iter().collect();
    for event in [
        "playlist-updated",
        "player-state-changed",
        "downloads-updated",
    ] {
        if let Err(error) = app.emit(event, ()) {
            errors.push(format!("文件操作结束，但窗口通知失败：{error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[tauri::command]
pub async fn rename_playlist_file(
    app: AppHandle,
    state: State<'_, AppState>,
    target: PlaylistFile,
    name: String,
) -> Result<(), String> {
    change_file(app, state.inner().clone(), target, Some(name)).await
}

#[tauri::command]
pub async fn delete_playlist_file(
    app: AppHandle,
    state: State<'_, AppState>,
    target: PlaylistFile,
) -> Result<(), String> {
    change_file(app, state.inner().clone(), target, None).await
}

#[tauri::command]
pub async fn refresh_playlist(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<PlaylistSnapshot, String> {
    let state = state.inner().clone();
    let task_state = state.clone();
    let update = tauri::async_runtime::spawn_blocking(move || {
        crate::services::directory_library::update_directory(&task_state, None)
    })
    .await
    .map_err(|error| error.to_string())??;
    super::settings::notify_directory_update(&app, update)?;
    application::playlist_snapshot(&state)
}

#[tauri::command]
pub async fn add_local_files(
    paths: Vec<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    application::add_local_paths(
        state.inner().clone(),
        app_handle,
        paths.into_iter().map(Into::into).collect(),
    )
    .await
}

/// 打开文件选择对话框并添加本地文件
#[tauri::command]
pub async fn pick_and_add_local_files(
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
        application::add_local_paths(state.inner().clone(), app_handle, paths).await?;
    }

    Ok(())
}

/// 打开文件夹选择对话框并递归添加媒体文件
#[tauri::command]
pub async fn pick_and_add_folder(
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
        application::add_local_paths(state.inner().clone(), app_handle, paths).await?;
    }

    Ok(())
}

/// 获取文件夹树结构
#[tauri::command]
pub fn get_folder_tree(folder_path: String) -> Result<LibraryItem, String> {
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
                } else if path.is_file() && media_capabilities::is_supported_media_path(&path) {
                    let id = path.to_string_lossy().to_string();
                    let title = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    children.push(LibraryItem::Track {
                        id,
                        title,
                        media_type: media_capabilities::media_type_from_path(&path),
                        source: crate::models::library::LibrarySource::Local { path: path.clone() },
                        parent: path.parent().map(|p| p.to_path_buf()),
                    });
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
pub fn get_media_subtitles(
    media_id: String,
    state: State<AppState>,
) -> Result<Vec<MediaAsset>, String> {
    let media = lock(&state.database)?.media(&media_id)?;
    let assets: Vec<_> = media
        .assets
        .into_iter()
        .filter(|asset| asset.kind == AssetKind::Subtitle)
        .collect();
    for asset in &assets {
        if !asset.path.is_file() {
            return Err(format!(
                "Subtitle file is unavailable: {}",
                asset.path.display()
            ));
        }
    }
    Ok(assets)
}

#[tauri::command]
pub fn remove_track(
    item_id: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    remove_and_notify(&state, &app_handle, Some(&item_id))
}
#[tauri::command]
pub fn clear_playlist(state: State<AppState>, app_handle: AppHandle) -> Result<(), String> {
    remove_and_notify(&state, &app_handle, None)
}
pub fn remove_and_notify(
    state: &AppState,
    app: &AppHandle,
    id: Option<&str>,
) -> Result<(), String> {
    let result = application::remove_entries(state, id);
    app.emit("playlist-updated", ())
        .map_err(|error| error.to_string())?;
    app.emit("player-state-changed", ())
        .map_err(|error| error.to_string())?;
    result
}
