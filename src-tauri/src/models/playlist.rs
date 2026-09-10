use crate::models::media::{Media, MediaOrigin, MediaType};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct PlaylistEntry {
    pub id: String,
    pub media_id: String,
    pub added_at: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    NotDownloaded,
    Downloaded,
}

// A read model for the list UI; never accepted as a media write command.
#[derive(Serialize)]
pub struct PlaylistItemView {
    pub id: String,
    pub media_id: String,
    pub canonical_key: String,
    pub title: String,
    pub media_type: MediaType,
    pub origin: MediaOrigin,
    pub cached_path: Option<PathBuf>,
    pub download_status: DownloadStatus,
    pub added_at: u64,
}

impl PlaylistItemView {
    pub fn new(entry: PlaylistEntry, media: Media) -> Self {
        let cached_path = media.cached_path().map(PathBuf::from);
        let status = if media.local_path().is_some_and(|path| path.is_file()) {
            DownloadStatus::Downloaded
        } else {
            DownloadStatus::NotDownloaded
        };
        Self {
            id: entry.id,
            media_id: media.id,
            canonical_key: media.canonical_key,
            title: media.title,
            media_type: media.media_type,
            origin: media.origin,
            cached_path,
            download_status: status,
            added_at: entry.added_at,
        }
    }
}

#[derive(Serialize)]
pub struct PlaylistSnapshot {
    pub revision: u64,
    pub items: Vec<PlaylistItemView>,
}

pub fn adjacent_entry(
    entries: &[PlaylistEntry],
    current: Option<&str>,
    mode: &str,
    backwards: bool,
) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let index = current.and_then(|id| entries.iter().position(|entry| entry.id == id));
    let next = match index {
        None => {
            if backwards {
                entries.len() - 1
            } else {
                0
            }
        }
        Some(index) => match mode {
            "repeat_one" => index,
            "random" if entries.len() > 1 => {
                let offset =
                    (uuid::Uuid::new_v4().as_u128() % (entries.len() - 1) as u128) as usize + 1;
                (index + offset) % entries.len()
            }
            "random" => index,
            "repeat_all" => (index + if backwards { entries.len() - 1 } else { 1 }) % entries.len(),
            _ if backwards => index.checked_sub(1)?,
            _ if index + 1 < entries.len() => index + 1,
            _ => return None,
        },
    };
    Some(entries[next].id.clone())
}
