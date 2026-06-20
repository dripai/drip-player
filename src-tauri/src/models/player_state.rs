use crate::models::playlist::{LibraryItem, PlaylistEntry, MediaType, Playlist};
use crate::services::audio_wrapper::AudioWrapper;
use crate::services::persistence::PersistenceManager;
use std::process::Child;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use serde::Serialize;

#[derive(Serialize)]
pub struct PlayerState {
    pub is_playing: bool,
    pub progress: f32,
    pub duration: f64,
    pub current_index: Option<usize>,
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

    // New data model: library + playlist entries
    pub library: Vec<LibraryItem>,
    pub playlist_entries: Vec<PlaylistEntry>,
    pub current_playlist_index: Option<usize>,
    pub playlist: Playlist,

    pub audio: AudioWrapper,
    pub video_process: Option<Child>,

    pub current_media_path: Option<PathBuf>,
    pub current_media_type: Option<MediaType>,

    // Item playing directly (not in playlist)
    pub temporary_item: Option<LibraryItem>,

    // Settings
    pub minimize_to_tray: bool,
}

impl MusicPlayer {
    pub fn new() -> Self {
        let library = PersistenceManager::load_library();
        let playlist_entries = PersistenceManager::load_playlist_entries();
        let mut playlist = Playlist::new();
        // 保留对旧播放列表的读取以便迁移，但新流程使用 library + playlist_entries
        playlist.tracks = PersistenceManager::load_playlist();
        let settings = PersistenceManager::load_settings();

        Self {
            is_playing: false,
            progress: 0.0,
            duration: Duration::from_secs(0),
            volume: 1.0,

            playback_start: None,
            playback_offset: Duration::from_secs(0),

            library,
            playlist_entries,
            playlist,
            current_playlist_index: None,

            audio: AudioWrapper::new(),
            video_process: None,
            current_media_path: None,
            current_media_type: None,
            temporary_item: None,
            minimize_to_tray: settings.minimize_to_tray,
        }
    }
}
