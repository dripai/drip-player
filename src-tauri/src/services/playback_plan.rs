use crate::models::media::Media;
use crate::models::playback::PlaybackPlan;
use crate::services::{media_probe, media_remux};
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
    plan_from_probe(path, &media_probe::probe_required(path)?)
}

fn plan_from_probe(path: &Path, info: &media_probe::MediaInfo) -> Result<PlaybackPlan, String> {
    if info.has_video {
        if media_probe::is_browser_native(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: path.to_path_buf(),
                video_codec: info.video_codec.clone().ok_or("视频缺少编码信息")?,
            });
        }

        if media_probe::can_remux_to_browser_mp4(info) {
            return Ok(PlaybackPlan::BrowserVideo {
                path: media_remux::ensure_mp4_remux(path)?,
                video_codec: info.video_codec.clone().ok_or("视频缺少编码信息")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hevc_mp4_uses_the_embedded_player_without_mpv_or_transcoding() {
        for codec in ["h264", "hevc", "av1"] {
            let info = media_probe::parse_output(
                &serde_json::to_vec(&serde_json::json!({
                    "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"5096.45"},
                    "streams":[{"codec_type":"video","codec_name":codec,"width":1920,"height":1080},
                        {"codec_type":"audio","codec_name":"aac"}]
                }))
                .unwrap(),
            )
            .unwrap();
            let path = Path::new("film.mp4");
            assert_eq!(
                plan_from_probe(path, &info).unwrap(),
                PlaybackPlan::BrowserVideo {
                    path: path.into(),
                    video_codec: codec.into(),
                }
            );
        }
    }

    #[test]
    #[ignore = "requires bundled ffprobe and SHADOW_TEST_MEDIA_FILE pointing to a real HEVC MP4"]
    fn real_hevc_file_uses_the_embedded_player() {
        let path = std::env::var_os("SHADOW_TEST_MEDIA_FILE").expect("SHADOW_TEST_MEDIA_FILE");
        let path = Path::new(&path);
        let plan = plan_for_local_path(path).unwrap();
        assert_eq!(
            plan,
            PlaybackPlan::BrowserVideo {
                path: path.into(),
                video_codec: "hevc".into()
            }
        );
    }

    #[test]
    #[ignore = "requires bundled ffprobe"]
    fn broken_mp4_reports_probe_failure_without_selecting_mpv() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("broken.mp4");
        std::fs::write(&path, b"not an mp4").unwrap();
        let error = plan_for_local_path(&path).unwrap_err();
        assert!(error.contains("媒体检测失败"));
        assert!(!error.contains("MPV"));
    }
}
