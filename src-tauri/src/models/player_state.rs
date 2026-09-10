use crate::models::playlist::{
    LibraryItem, MediaType, PlaylistDownloadStatus, PlaylistItem, PlaylistItemView, PlaylistOrigin,
    PlaylistSnapshot,
};
use crate::services::audio_wrapper::AudioWrapper;
use crate::services::persistence::{AppSettings, PersistenceManager};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

#[derive(Serialize)]
pub struct PlayerState {
    pub is_playing: bool,
    pub progress: f32,
    pub duration: f64,
    pub current_item_id: Option<String>,
    pub current_item: Option<LibraryItem>,
}

pub struct MusicPlayer {
    pub is_playing: bool,
    pub progress: f32,
    pub duration: Duration,
    pub volume: f32,

    // Time tracking for progress calculation
    pub playback_start: Option<Instant>,
    pub playback_offset: Duration, // Used for seek and pause/resume accumulation

    pub playlist_items: Vec<PlaylistItem>,
    pub playlist_revision: u64,
    // Database revisions exclude transient download status changes.
    pub playlist_storage_revision: u64,
    pub current_playlist_item_id: Option<String>,
    pub downloading_item_ids: HashSet<String>,

    pub audio: AudioWrapper,
    pub video_process: Option<Child>,

    pub current_media_path: Option<PathBuf>,
    pub current_media_type: Option<MediaType>,

    // Item playing directly (not in playlist)
    pub temporary_item: Option<LibraryItem>,

    // Settings
    pub settings: AppSettings,
    pub persistence: PersistenceManager,
}

impl MusicPlayer {
    pub fn new() -> Result<Self, String> {
        let persistence = PersistenceManager::open()?;
        let playlist_state = persistence.load_playlist()?;
        let settings = persistence.load_settings()?;

        Ok(Self {
            is_playing: false,
            progress: 0.0,
            duration: Duration::from_secs(0),
            volume: 1.0,

            playback_start: None,
            playback_offset: Duration::from_secs(0),

            playlist_items: playlist_state.items,
            playlist_revision: playlist_state.revision,
            playlist_storage_revision: playlist_state.revision,
            current_playlist_item_id: None,
            downloading_item_ids: HashSet::new(),

            audio: AudioWrapper::new(),
            video_process: None,
            current_media_path: None,
            current_media_type: None,
            temporary_item: None,
            settings,
            persistence,
        })
    }

    pub fn playlist_snapshot(&self) -> PlaylistSnapshot {
        let items = self
            .playlist_items
            .iter()
            .map(|item| {
                let download_status = if self.downloading_item_ids.contains(&item.id) {
                    PlaylistDownloadStatus::Downloading
                } else if matches!(item.origin, PlaylistOrigin::Local { .. })
                    || item
                        .cached_path
                        .as_ref()
                        .map(|path| path.exists())
                        .unwrap_or(false)
                {
                    PlaylistDownloadStatus::Downloaded
                } else {
                    PlaylistDownloadStatus::NotDownloaded
                };
                PlaylistItemView {
                    id: item.id.clone(),
                    media_id: item.media_id.clone(),
                    canonical_key: item.canonical_key.clone(),
                    title: item.title.clone(),
                    media_type: item.media_type.clone(),
                    origin: item.origin.clone(),
                    cached_path: item.cached_path.clone(),
                    download_status,
                    added_at: item.added_at,
                }
            })
            .collect();

        PlaylistSnapshot {
            revision: self.playlist_revision,
            items,
        }
    }
}
