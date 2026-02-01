use crate::services::fs::{FsEntry, list_dir, move_path, remove_path, new_folder, ensure_dir, write_binary, read_text, write_text};

#[tauri::command]
pub fn fs_list_dir(path: String) -> Result<Vec<FsEntry>, String> { list_dir(path) }

#[tauri::command]
pub fn fs_move(src_path: String, dest_dir: String, new_name: Option<String>) -> Result<(), String> { move_path(src_path, dest_dir, new_name) }

#[tauri::command]
pub fn fs_remove(path: String) -> Result<(), String> { remove_path(path) }

#[tauri::command]
pub fn fs_new_folder(parent_dir: String, name: String) -> Result<(), String> { new_folder(parent_dir, name) }

#[tauri::command]
pub fn fs_ensure_dir(path: String) -> Result<(), String> { ensure_dir(path) }

#[tauri::command]
pub fn fs_write_binary(path: String, bytes: Vec<u8>) -> Result<(), String> { write_binary(path, bytes) }

#[tauri::command]
pub fn fs_read_text(path: String) -> Result<String, String> { read_text(path) }

#[tauri::command]
pub fn fs_write_text(path: String, content: String) -> Result<(), String> { write_text(path, content) }
