use crate::models::playlist::MediaType;
use crate::services::media_capabilities;
use crate::services::toolchain;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub media_type: MediaType,
    pub duration_secs: Option<f64>,
    pub container: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    format: Option<FfprobeFormat>,
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
}

pub fn probe(path: &Path) -> Option<MediaInfo> {
    let output = toolchain::hidden_command(&toolchain::ffprobe_path())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        println!("ffprobe failed for {:?}", path);
        return None;
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout).ok()?;
    let video_codec = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .and_then(|stream| stream.codec_name.clone());
    let audio_codec = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .and_then(|stream| stream.codec_name.clone());
    let duration_secs = parsed
        .format
        .as_ref()
        .and_then(|format| format.duration.as_ref())
        .and_then(|duration| duration.parse::<f64>().ok());
    let container = parsed.format.and_then(|format| format.format_name);
    let has_video = video_codec.is_some();
    let has_audio = audio_codec.is_some();
    let media_type = if has_video {
        MediaType::Video
    } else if has_audio {
        MediaType::Audio
    } else {
        media_capabilities::media_type_from_path(path)
    };

    Some(MediaInfo {
        media_type,
        duration_secs,
        container,
        video_codec,
        audio_codec,
        has_video,
        has_audio,
    })
}

pub fn duration(path: &Path) -> Option<Duration> {
    probe(path)
        .and_then(|info| info.duration_secs)
        .map(Duration::from_secs_f64)
}

pub fn is_browser_native(info: &MediaInfo) -> bool {
    let container = info.container.as_deref().unwrap_or_default();
    let video_codec = info.video_codec.as_deref().unwrap_or_default();
    let audio_codec = info.audio_codec.as_deref().unwrap_or_default();

    if container.contains("webm") {
        return container.contains("webm")
            && matches!(video_codec, "vp8" | "vp9" | "av1")
            && (audio_codec.is_empty() || matches!(audio_codec, "opus" | "vorbis"));
    }

    is_mp4_family_container(container)
        && matches!(video_codec, "h264" | "av1")
        && (audio_codec.is_empty() || matches!(audio_codec, "aac" | "mp3" | "alac" | "opus"))
}

pub fn can_remux_to_browser_mp4(info: &MediaInfo) -> bool {
    matches!(info.video_codec.as_deref(), Some("h264"))
        && matches!(info.audio_codec.as_deref(), None | Some("aac") | Some("mp3"))
}

fn is_mp4_family_container(container: &str) -> bool {
    container.split(',').any(|part| {
        matches!(
            part.trim(),
            "mp4" | "mov" | "m4a" | "m4v" | "3gp" | "3g2" | "mj2"
        )
    })
}
