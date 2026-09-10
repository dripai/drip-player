use std::path::Path;

pub fn stamp(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("不是普通文件：{}", path.display()));
    }
    let modified = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(format!("{}:{modified}", metadata.len()))
}

#[cfg(windows)]
pub fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
    };
    let from_path = std::fs::canonicalize(from).map_err(|e| e.to_string())?;
    let to_path = std::fs::canonicalize(to.parent().ok_or("文件缺少父目录")?)
        .map_err(|e| e.to_string())?
        .join(to.file_name().ok_or("文件缺少名称")?);
    let from_wide: Vec<_> = from_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<_> = to_path.as_os_str().encode_wide().chain(Some(0)).collect();
    // No REPLACE_EXISTING and no attribute changes. Both NUL-terminated buffers
    // remain alive throughout this synchronous Windows call.
    unsafe {
        MoveFileExW(
            PCWSTR(from_wide.as_ptr()),
            PCWSTR(to_wide.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|error| format!("无法移动 {} → {}：{error}", from.display(), to.display()))
}

#[cfg(not(windows))]
pub fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    let mut path =
        tempfile::TempPath::try_from_path(from.to_path_buf()).map_err(|e| e.to_string())?;
    // These are existing files: an unsuccessful move must not delete its source.
    path.disable_cleanup(true);
    path.persist_noclobber(to).map_err(|e| {
        format!(
            "无法移动 {} → {}：{}",
            from.display(),
            to.display(),
            e.error
        )
    })?;
    if from.try_exists().map_err(|e| e.to_string())? {
        if let Err(error) = std::fs::remove_file(from) {
            return match std::fs::remove_file(to) {
                Ok(()) => Err(format!("无法移除旧文件名 {}：{error}", from.display())),
                Err(cleanup) => Err(format!(
                    "移动未完成，两个文件名均保留：{}、{}；{error}；{cleanup}",
                    from.display(),
                    to.display()
                )),
            };
        }
    }
    Ok(())
}
