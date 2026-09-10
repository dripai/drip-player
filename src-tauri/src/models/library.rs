use crate::models::media::MediaType;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
