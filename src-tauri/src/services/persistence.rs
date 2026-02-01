use std::fs;
use std::path::PathBuf;
use crate::models::playlist::Track;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub minimize_to_tray: bool,
}

pub struct PersistenceManager;

impl PersistenceManager {
    fn config_dir() -> PathBuf {
        // Use executable's directory for config
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        exe_dir.join("config")
    }

    fn playlist_path() -> PathBuf {
        Self::config_dir().join("playlist.json")
    }

    fn downloads_path() -> PathBuf {
        Self::config_dir().join("downloads.json")
    }

    fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn ensure_config_dir() {
        let dir = Self::config_dir();
        if !dir.exists() {
            let _ = fs::create_dir_all(dir);
        }
    }

    pub fn save_playlist(tracks: &[Track]) {
        Self::ensure_config_dir();
        if let Ok(json) = serde_json::to_string_pretty(tracks) {
            let _ = fs::write(Self::playlist_path(), json);
        }
    }

    pub fn load_playlist() -> Vec<Track> {
        let path = Self::playlist_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(tracks) = serde_json::from_str(&content) {
                    return tracks;
                }
            }
        }
        Vec::new()
    }

    pub fn save_downloads(tracks: &[Track]) {
        Self::ensure_config_dir();
        if let Ok(json) = serde_json::to_string_pretty(tracks) {
            let _ = fs::write(Self::downloads_path(), json);
        }
    }

    pub fn load_downloads() -> Vec<Track> {
        let path = Self::downloads_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(tracks) = serde_json::from_str(&content) {
                    return tracks;
                }
            }
        }
        Vec::new()
    }

    pub fn save_settings(settings: &AppSettings) {
        Self::ensure_config_dir();
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            let _ = fs::write(Self::settings_path(), json);
        }
    }

    pub fn load_settings() -> AppSettings {
        let path = Self::settings_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(settings) = serde_json::from_str(&content) {
                    return settings;
                }
            }
        }
        AppSettings::default()
    }

    pub fn scan_cache_for_tracks() -> Vec<Track> {
        // Use executable's directory for cache
        let cache_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join("cache");
        let mut tracks = Vec::new();

        if cache_dir.exists() {
            if let Ok(entries) = fs::read_dir(&cache_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_file() {
                            // Check extension
                            if let Some(ext) = path.extension() {
                                let ext_str = ext.to_string_lossy().to_lowercase();
                                if ["mp3", "flac", "wav", "ogg", "m4a", "mp4", "webm", "mkv", "avi", "mov"].contains(&ext_str.as_str()) {
                                    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                                    // Exclude temp files
                                    if !file_name.contains(".tmp.") && !file_name.ends_with(".tmp") && 
                                       !file_name.contains(".part") && !file_name.ends_with(".ytdl") {
                                         tracks.push(Track::new_local(path));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        tracks
    }
}
