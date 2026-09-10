use crate::services::toolchain;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

const REMUX_CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const REMUX_CACHE_MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;

fn output_lock(path: &Path) -> Result<Arc<Mutex<()>>, String> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}

pub fn ensure_mp4_remux(input: &Path) -> Result<PathBuf, String> {
    cleanup_remux_cache();

    let output = remux_output_path(input)?;
    let output_lock = output_lock(&output)?;
    let _preparing = output_lock.lock().map_err(|error| error.to_string())?;
    if output.exists() {
        touch_cache_file(&output);
        return Ok(output);
    }

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create remux cache dir: {}", e))?;
    }

    // FFmpeg writes a private file. Only a completed remux becomes visible to other sessions.
    let pending = tempfile::Builder::new()
        .prefix(".shadow-remux-")
        .suffix(".part")
        .tempfile_in(
            output
                .parent()
                .ok_or("Remux output has no parent directory")?,
        )
        .map_err(|error| error.to_string())?
        .into_temp_path();

    let status = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-map", "0:v:0"])
        .args(["-map", "0:a:0?"])
        .args(["-c", "copy"])
        .args(["-movflags", "+faststart"])
        .args(["-f", "mp4"])
        .arg(&pending)
        .status()
        .map_err(|e| format!("Failed to start ffmpeg remux: {}", e))?;

    if status.success() {
        pending
            .persist_noclobber(&output)
            .map_err(|error| format!("Cannot publish remux: {error}"))?;
        touch_cache_file(&output);
        Ok(output)
    } else {
        Err(format!("ffmpeg remux failed for {}", input.display()))
    }
}

fn remux_output_path(input: &Path) -> Result<PathBuf, String> {
    let metadata =
        std::fs::metadata(input).map_err(|e| format!("Failed to read media metadata: {}", e))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();

    let mut hasher = DefaultHasher::new();
    input.to_string_lossy().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);

    let cache_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("cache")
        .join("remux");

    Ok(cache_dir.join(format!("{:016x}.mp4", hasher.finish())))
}

pub fn cleanup_remux_cache() {
    let cache_dir = remux_cache_dir();
    let Ok(entries) = fs::read_dir(&cache_dir) else {
        return;
    };

    let now = SystemTime::now();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("mp4") {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        let accessed = metadata
            .accessed()
            .or_else(|_| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let age = now.duration_since(accessed).unwrap_or_default();

        if age > REMUX_CACHE_MAX_AGE {
            let _ = fs::remove_file(&path);
            continue;
        }

        files.push((path, metadata.len(), accessed));
    }

    let mut total_size: u64 = files.iter().map(|(_, len, _)| *len).sum();
    if total_size <= REMUX_CACHE_MAX_BYTES {
        return;
    }

    files.sort_by_key(|(_, _, accessed)| *accessed);
    for (path, len, _) in files {
        if total_size <= REMUX_CACHE_MAX_BYTES {
            break;
        }

        if fs::remove_file(&path).is_ok() {
            total_size = total_size.saturating_sub(len);
        }
    }
}

fn remux_cache_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("cache")
        .join("remux")
}

fn touch_cache_file(path: &Path) {
    let _ = fs::OpenOptions::new().append(true).open(path);
}
