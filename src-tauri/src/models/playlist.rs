use std::path::PathBuf;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Source of a library item
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LibrarySource {
    Local { path: PathBuf },
    Remote { url: String, id: String, cached_path: Option<PathBuf>, #[serde(default = "default_media_type")] media_type: MediaType, download_status: DownloadStatus },
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

/// Playlist entry is a lightweight reference to a LibraryItem
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub id: String,      // entry id
    pub item_id: String, // referenced LibraryItem id
    pub added_at: u64,   // unix timestamp
}

/// Legacy Track and Playlist kept for migration helpers (not used by new codepaths)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TrackSource {
    Local(PathBuf),
    Remote { url: String, id: String, cached_path: Option<PathBuf>, title: String, #[serde(default = "default_media_type")] media_type: MediaType, is_downloading: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub source: TrackSource,
}

impl Track {
    pub fn new_local(path: PathBuf) -> Self {
        Self { source: TrackSource::Local(path) }
    }

    pub fn new_remote(url: String, id: String, title: String, _duration: Option<Duration>, media_type: MediaType) -> Self {
        Self { source: TrackSource::Remote { url, id, cached_path: None, title, media_type, is_downloading: false } }
    }

    pub fn name(&self) -> String {
        match &self.source {
            TrackSource::Local(path) => path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            TrackSource::Remote { title, .. } => title.clone(),
        }
    }
}

/// Legacy playlist structure for migration helpers
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Playlist {
    pub tracks: Vec<Track>,
    pub current_index: Option<usize>,
}

impl Playlist {
    pub fn new() -> Self {
        Self { tracks: Vec::new(), current_index: None }
    }

    /// Convert legacy tracks to PlaylistEntry referencing by id/path
    pub fn to_entries(&self) -> Vec<PlaylistEntry> {
        self.tracks.iter().map(|t| {
            let item_id = match &t.source {
                TrackSource::Local(p) => p.to_string_lossy().to_string(),
                TrackSource::Remote { id, .. } => id.clone(),
            };
            PlaylistEntry { id: Uuid::new_v4().to_string(), item_id, added_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
        }).collect()
    }

    /// Helper to convert a Track into a LibraryItem.Track
    pub fn track_to_library_item(t: &Track) -> LibraryItem {
        match &t.source {
            TrackSource::Local(p) => LibraryItem::Track { id: p.to_string_lossy().to_string(), title: p.file_stem().unwrap_or_default().to_string_lossy().to_string(), media_type: default_media_type(), source: LibrarySource::Local { path: p.clone() }, parent: p.parent().map(|pp| pp.to_path_buf()) },
            TrackSource::Remote { url, id, cached_path, title, media_type, .. } => LibraryItem::Track { id: id.clone(), title: title.clone(), media_type: media_type.clone(), source: LibrarySource::Remote { url: url.clone(), id: id.clone(), cached_path: cached_path.clone(), media_type: media_type.clone(), download_status: if cached_path.is_some() { DownloadStatus::Downloaded } else { DownloadStatus::NotDownloaded } }, parent: None },
        }
    }
}
