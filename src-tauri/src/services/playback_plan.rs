use crate::models::playlist::{LibraryItem, LibrarySource, MediaType};
use crate::services::{media_capabilities, media_probe, media_remux};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "engine", rename_all = "snake_case")]
pub enum PlaybackPlan {
    BrowserVideo { path: Option<PathBuf>, remote_url: Option<String> },
    ExternalVideo { path: PathBuf },
    Audio { path: PathBuf },
    RemotePending { url: String },
}

impl PlaybackPlan {
    pub fn media_type(&self) -> MediaType {
        match self {
            PlaybackPlan::Audio { .. } => MediaType::Audio,
            PlaybackPlan::BrowserVideo { .. }
            | PlaybackPlan::ExternalVideo { .. }
            | PlaybackPlan::RemotePending { .. } => MediaType::Video,
        }
    }

    pub fn local_path(&self) -> Option<&Path> {
        match self {
            PlaybackPlan::BrowserVideo { path: Some(path), .. }
            | PlaybackPlan::ExternalVideo { path }
            | PlaybackPlan::Audio { path } => Some(path.as_path()),
            _ => None,
        }
    }
}

pub fn plan_for_item(item: &LibraryItem) -> Result<PlaybackPlan, String> {
    let LibraryItem::Track {
        source, media_type, ..
    } = item
    else {
        return Err("Selected item is not a track".to_string());
    };

    match source {
        LibrarySource::Local { path } => plan_for_local_path(path),
        LibrarySource::Remote {
            url,
            cached_path,
            media_type: source_media_type,
            ..
        } => {
            if let Some(path) = cached_path {
                plan_for_cached_path(path, source_media_type)
            } else if *media_type == MediaType::Video || *source_media_type == MediaType::Video {
                Ok(PlaybackPlan::RemotePending { url: url.clone() })
            } else {
                Ok(PlaybackPlan::Audio {
                    path: PathBuf::from(url),
                })
            }
        }
    }
}

fn plan_for_local_path(path: &Path) -> Result<PlaybackPlan, String> {
    if let Some(info) = media_probe::probe(path) {
        return plan_from_probe(path, &info);
    }

    plan_from_extension(path)
}

fn plan_for_cached_path(path: &Path, media_type: &MediaType) -> Result<PlaybackPlan, String> {
    if let Some(info) = media_probe::probe(path) {
        return plan_from_probe(path, &info);
    }

    if *media_type == MediaType::Video {
        return Ok(PlaybackPlan::ExternalVideo {
            path: path.to_path_buf(),
        });
    }

    plan_from_extension(path)
}

fn plan_from_probe(path: &Path, info: &media_probe::MediaInfo) -> Result<PlaybackPlan, String> {
    if info.has_video {
        if media_probe::is_browser_native(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: Some(path.to_path_buf()),
                remote_url: None,
            });
        }

        if media_probe::can_remux_to_browser_mp4(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: Some(media_remux::ensure_mp4_remux(path)?),
                remote_url: None,
            });
        }

        return Ok(PlaybackPlan::ExternalVideo {
            path: path.to_path_buf(),
        });
    }

    if info.has_audio {
        return Ok(PlaybackPlan::Audio {
            path: path.to_path_buf(),
        });
    }

    Err(format!("Unsupported media stream layout: {}", path.display()))
}

fn plan_from_extension(path: &Path) -> Result<PlaybackPlan, String> {
    if media_capabilities::is_video_path(path) {
        Ok(PlaybackPlan::ExternalVideo {
            path: path.to_path_buf(),
        })
    } else if media_capabilities::is_audio_path(path) {
        Ok(PlaybackPlan::Audio {
            path: path.to_path_buf(),
        })
    } else {
        Err(format!("Unsupported media format: {}", path.display()))
    }
}
