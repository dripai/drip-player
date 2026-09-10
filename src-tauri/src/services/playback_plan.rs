use crate::models::media::Media;
use crate::models::playback::PlaybackPlan;
use crate::services::{media_capabilities, media_probe, media_remux};
use std::path::Path;

// Runs in a blocking preparation task, never while holding playback state.
pub fn prepare(media: &Media) -> Result<PlaybackPlan, String> {
    let path = media
        .local_path()
        .ok_or("Media has no local playback asset")?;
    if !path.is_file() {
        return Err(format!("Media file is unavailable: {}", path.display()));
    }
    plan_for_local_path(path)
}

fn plan_for_local_path(path: &Path) -> Result<PlaybackPlan, String> {
    if let Some(info) = media_probe::probe(path) {
        return plan_from_probe(path, &info);
    }

    plan_from_extension(path)
}

fn plan_from_probe(path: &Path, info: &media_probe::MediaInfo) -> Result<PlaybackPlan, String> {
    if info.has_video {
        if media_probe::is_browser_native(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: path.to_path_buf(),
            });
        }

        if media_probe::can_remux_to_browser_mp4(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: media_remux::ensure_mp4_remux(path)?,
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

    Err(format!(
        "Unsupported media stream layout: {}",
        path.display()
    ))
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
