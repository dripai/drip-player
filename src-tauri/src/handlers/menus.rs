use tauri::{
    menu::{ContextMenu, Menu, MenuItem},
    Manager, Window,
};

/// 显示曲目上下文菜单
#[tauri::command]
pub async fn show_track_context_menu(
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
pub async fn show_playlist_context_menu(window: Window, locale: String) -> Result<(), String> {
    let app_handle = window.app_handle().clone();

    let clear_playlist_label = if locale == "zh" {
        "清空播放列表"
    } else {
        "Clear entire playlist"
    };

    let clear_playlist_item = MenuItem::with_id(
        &app_handle,
        "clear_playlist",
        clear_playlist_label,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(&app_handle, &[&clear_playlist_item]).map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())?;
    Ok(())
}
