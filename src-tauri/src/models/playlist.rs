use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
}

fn default_media_type() -> MediaType {
    MediaType::Audio
}

/// Download status for remote resources
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DownloadStatus {
    NotDownloaded,
    Downloading,
    Downloaded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaylistOrigin {
    Local {
        path: PathBuf,
    },
    Remote {
        url: String,
        provider: String,
        external_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlaylistItem {
    pub id: String,
    pub media_id: String,
    pub canonical_key: String,
    pub title: String,
    pub media_type: MediaType,
    pub origin: PlaylistOrigin,
    pub cached_path: Option<PathBuf>,
    pub added_at: u64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistDownloadStatus {
    NotDownloaded,
    Downloading,
    Downloaded,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaylistItemView {
    pub id: String,
    pub media_id: String,
    pub canonical_key: String,
    pub title: String,
    pub media_type: MediaType,
    pub origin: PlaylistOrigin,
    pub cached_path: Option<PathBuf>,
    pub download_status: PlaylistDownloadStatus,
    pub added_at: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaylistSnapshot {
    pub revision: u64,
    pub items: Vec<PlaylistItemView>,
}

impl PlaylistItem {
    pub fn to_library_item(&self) -> LibraryItem {
        let source = match &self.origin {
            PlaylistOrigin::Local { path } => LibrarySource::Local { path: path.clone() },
            PlaylistOrigin::Remote {
                url, external_id, ..
            } => LibrarySource::Remote {
                url: url.clone(),
                id: external_id.clone(),
                cached_path: self.cached_path.clone(),
                media_type: self.media_type.clone(),
                download_status: if self.cached_path.is_some() {
                    DownloadStatus::Downloaded
                } else {
                    DownloadStatus::NotDownloaded
                },
            },
        };

        LibraryItem::Track {
            id: self.id.clone(),
            title: self.title.clone(),
            media_type: self.media_type.clone(),
            source,
            parent: match &self.origin {
                PlaylistOrigin::Local { path } => path.parent().map(Path::to_path_buf),
                PlaylistOrigin::Remote { .. } => None,
            },
        }
    }
}

pub fn canonical_local_identity(path: &Path) -> (PathBuf, String) {
    let normalized_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normalized = normalized_path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized = normalized.to_lowercase();
    }
    (normalized_path, format!("local:{}", normalized))
}

pub fn canonical_remote_key(provider: &str, external_id: &str) -> String {
    format!("remote:{}:{}", provider.to_lowercase(), external_id)
}

pub fn provider_key_for_url(value: &str) -> String {
    let lower = value.to_lowercase();
    if lower.contains("bilibili.com") || lower.contains("b23.tv") {
        return "bilibili".to_string();
    }
    if lower.contains("youtube.com") || lower.contains("youtu.be") {
        return "youtube".to_string();
    }
    if lower.contains("douyin.com") || lower.contains("iesdouyin.com") {
        return "douyin".to_string();
    }
    if lower.contains("v.qq.com") || lower.contains("qq.com/x/cover") {
        return "tencent".to_string();
    }
    if lower.contains("channels.weixin.qq.com") || lower.contains("finder.video.qq.com") {
        return "weixin".to_string();
    }

    url::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_else(|| "generic".to_string())
}

/// Source of a library item
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LibrarySource {
    Local {
        path: PathBuf,
    },
    Remote {
        url: String,
        id: String,
        cached_path: Option<PathBuf>,
        #[serde(default = "default_media_type")]
        media_type: MediaType,
        download_status: DownloadStatus,
    },
}

/// Unified resource in the library (can be a folder or a track)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LibraryItem {
    Track {
        id: String,
        title: String,
        media_type: MediaType,
        source: LibrarySource,
        parent: Option<PathBuf>,
    },
    Folder {
        name: String,
        path: PathBuf,
        children: Vec<LibraryItem>,
    },
}
