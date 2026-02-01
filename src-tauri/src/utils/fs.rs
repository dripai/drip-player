use std::{fs, path::Path};

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    if !dst.exists() { fs::create_dir_all(dst).map_err(|e| e.to_string())?; }
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() { copy_dir_all(&from, &to)?; } else { fs::copy(&from, &to).map_err(|e| e.to_string())?; }
    }
    Ok(())
}
