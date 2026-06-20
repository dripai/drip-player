use crate::models::playlist::MediaType;
use std::path::Path;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus"];
pub const BROWSER_VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "webm"];
pub const EXTERNAL_VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "avi", "mov", "flv", "wmv", "ts", "m2ts", "mpg", "mpeg", "3gp",
];
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "mp4", "m4v", "webm", "mkv",
    "avi", "mov", "flv", "wmv", "ts", "m2ts", "mpg", "mpeg", "3gp",
];

pub fn extension_lower(path: &Path) -> Option<String> {
    path.extension().map(|ext| ext.to_string_lossy().to_lowercase())
}

pub fn is_audio_path(path: &Path) -> bool {
    extension_lower(path)
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

pub fn is_browser_video_path(path: &Path) -> bool {
    extension_lower(path)
        .map(|ext| BROWSER_VIDEO_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

pub fn is_external_video_path(path: &Path) -> bool {
    extension_lower(path)
        .map(|ext| EXTERNAL_VIDEO_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

pub fn is_video_path(path: &Path) -> bool {
    is_browser_video_path(path) || is_external_video_path(path)
}

pub fn is_supported_media_path(path: &Path) -> bool {
    is_audio_path(path) || is_video_path(path)
}

pub fn media_type_from_path(path: &Path) -> MediaType {
    if is_video_path(path) {
        MediaType::Video
    } else {
        MediaType::Audio
    }
}
