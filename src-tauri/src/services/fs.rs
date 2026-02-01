use serde::Serialize;
use std::{fs, path::PathBuf};
use crate::utils::fs::copy_dir_all;

#[derive(Serialize)]
pub struct FsEntry { pub name: String, pub path: String, pub is_dir: bool }

pub fn list_dir(path: String) -> Result<Vec<FsEntry>, String> {
    let p = PathBuf::from(path);
    let mut out = Vec::new();
    let rd = fs::read_dir(&p).map_err(|e| e.to_string())?;
    for ent in rd {
        let ent = ent.map_err(|e| e.to_string())?;
        let name = ent.file_name().to_string_lossy().to_string();
        let child_path = ent.path().to_string_lossy().to_string();
        let md = ent.metadata().map_err(|e| e.to_string())?;
        out.push(FsEntry { name, path: child_path, is_dir: md.is_dir() });
    }
    Ok(out)
}

pub fn move_path(src_path: String, dest_dir: String, new_name: Option<String>) -> Result<(), String> {
    let src = PathBuf::from(src_path);
    let name = new_name.unwrap_or_else(|| src.file_name().unwrap().to_string_lossy().to_string());
    let dest = PathBuf::from(dest_dir).join(name);
    if fs::rename(&src, &dest).is_err() {
        let md = fs::metadata(&src).map_err(|e| e.to_string())?;
        if md.is_dir() { copy_dir_all(&src, &dest)?; fs::remove_dir_all(&src).map_err(|e| e.to_string())?; }
        else { fs::copy(&src, &dest).map_err(|e| e.to_string())?; fs::remove_file(&src).map_err(|e| e.to_string())?; }
    }
    Ok(())
}

pub fn remove_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(path);
    if p.is_dir() { fs::remove_dir_all(&p).map_err(|e| e.to_string())?; } else { fs::remove_file(&p).map_err(|e| e.to_string())?; }
    Ok(())
}

pub fn new_folder(parent_dir: String, name: String) -> Result<(), String> {
    let dir = PathBuf::from(parent_dir).join(name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?; Ok(())
}

pub fn ensure_dir(path: String) -> Result<(), String> {
    let p = PathBuf::from(path);
    fs::create_dir_all(&p).map_err(|e| e.to_string())?; Ok(())
}

pub fn write_binary(path: String, bytes: Vec<u8>) -> Result<(), String> {
    let p = PathBuf::from(path);
    if let Some(parent) = p.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    fs::write(&p, bytes).map_err(|e| e.to_string())?; Ok(())
}

pub fn read_text(path: String) -> Result<String, String> {
    fs::read_to_string(PathBuf::from(path)).map_err(|e| e.to_string())
}

pub fn write_text(path: String, content: String) -> Result<(), String> {
    let mut f = fs::File::create(PathBuf::from(path)).map_err(|e| e.to_string())?;
    use std::io::Write;
    f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}
