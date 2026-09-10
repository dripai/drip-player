use crate::models::media::Media;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "engine", rename_all = "snake_case")]
pub enum PlaybackPlan {
    BrowserVideo { path: PathBuf },
    ExternalVideo { path: PathBuf },
    Audio { path: PathBuf },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Preparing,
    Ready,
    Playing,
    Paused,
    Buffering,
    Ended,
    Stopped,
    Failed,
    External,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaybackSession {
    pub id: u64,
    pub media: Media,
    pub playlist_entry_id: Option<String>,
    pub plan: Option<PlaybackPlan>,
    pub status: PlaybackStatus,
    pub position: f64,
    pub duration: f64,
    pub error: Option<String>,
    #[serde(skip)]
    pub browser_sequence: u64,
    #[serde(skip)]
    pub browser_owner: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct PlaybackSnapshot {
    pub revision: u64,
    pub session: Option<PlaybackSession>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPlaybackReport {
    pub session_id: u64,
    pub sequence: u64,
    pub owner: String,
    pub position: f64,
    pub duration: f64,
    pub status: PlaybackStatus,
}

impl BrowserPlaybackReport {
    pub fn validate(&self) -> Result<(), String> {
        if !self.position.is_finite()
            || self.position < 0.0
            || !self.duration.is_finite()
            || self.duration < 0.0
        {
            return Err("Invalid playback time".into());
        }
        if !matches!(
            self.status,
            PlaybackStatus::Playing
                | PlaybackStatus::Paused
                | PlaybackStatus::Buffering
                | PlaybackStatus::Ended
        ) {
            return Err("Invalid browser playback status".into());
        }
        Ok(())
    }
}
