use crate::models::playlist::{Playlist, TrackSource, MediaType, Track};
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
    pub current_track: Option<Track>,
}

pub struct MusicPlayer {
    pub is_playing: bool,
    pub progress: f32,
    pub duration: Duration,
    pub volume: f32,
    
    // Time tracking for progress calculation
    pub playback_start: Option<Instant>,
    pub playback_offset: Duration, // Used for seek and pause/resume accumulation
    
    pub playlist: Playlist,
    pub audio: AudioWrapper,
    pub video_process: Option<Child>,
    
    pub current_media_path: Option<PathBuf>,
    pub current_media_type: Option<MediaType>,
}

impl MusicPlayer {
    pub fn new() -> Self {
        let mut playlist = Playlist::new();
        playlist.tracks = PersistenceManager::load_playlist();

        // Don't set current_index on startup - let user choose what to play
        // if !playlist.tracks.is_empty() {
        //     playlist.current_index = Some(0);
        // }

        Self {
            is_playing: false,
            progress: 0.0,
            duration: Duration::from_secs(0),
            volume: 1.0,

            playback_start: None,
            playback_offset: Duration::from_secs(0),

            playlist,
            audio: AudioWrapper::new(),
            video_process: None,
            current_media_path: None,
            current_media_type: None,
        }
    }
}
