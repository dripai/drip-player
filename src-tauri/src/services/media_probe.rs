use crate::models::media::MediaType;
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
    pub video_height: Option<u32>,
    pub video_width: Option<u32>,
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
    height: Option<u32>,
    width: Option<u32>,
    disposition: Option<FfprobeDisposition>,
}

#[derive(Debug, Deserialize)]
struct FfprobeDisposition {
    attached_pic: Option<u8>,
}

pub fn probe(path: &Path) -> Option<MediaInfo> {
    probe_required(path).ok()
}

pub fn probe_required(path: &Path) -> Result<MediaInfo, String> {
    let ffprobe = toolchain::find_tool("ffprobe")
        .ok_or("缺少媒体检测工具 ffprobe，请重新准备应用工具目录")?;
    let output = toolchain::hidden_command(&ffprobe.to_string_lossy())
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
        .map_err(|error| format!("无法检测媒体文件：{error}"))?;

    if !output.status.success() {
        return Err(format!(
            "媒体检测失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_output(&output.stdout)
}

pub fn parse_output(bytes: &[u8]) -> Result<MediaInfo, String> {
    let parsed: FfprobeOutput = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid ffprobe output: {error}"))?;
    let video = parsed.streams.iter().find(|stream| {
        stream.codec_type.as_deref() == Some("video")
            && stream
                .disposition
                .as_ref()
                .is_none_or(|value| value.attached_pic != Some(1))
    });
    let audio = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let video_codec = video.and_then(|stream| stream.codec_name.clone());
    let audio_codec = audio.and_then(|stream| stream.codec_name.clone());
    let duration_secs = parsed
        .format
        .as_ref()
        .and_then(|format| format.duration.as_ref())
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0);
    let container = parsed.format.and_then(|format| format.format_name);
    let has_video = video.is_some();
    let has_audio = audio.is_some();
    let media_type = if has_video {
        MediaType::Video
    } else if has_audio {
        MediaType::Audio
    } else {
        return Err("No audio or video streams found".into());
    };

    Ok(MediaInfo {
        media_type,
        duration_secs,
        container,
        video_codec,
        audio_codec,
        has_video,
        has_audio,
        video_height: video.and_then(|stream| stream.height),
        video_width: video.and_then(|stream| stream.width),
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
        // HEVC decoding is provided by the WebView/OS. Let the actual video
        // element report unsupported decoding instead of forcing external MPV.
        && matches!(video_codec, "h264" | "av1" | "hevc")
        && (audio_codec.is_empty() || matches!(audio_codec, "aac" | "mp3" | "alac" | "opus"))
}

pub fn can_remux_to_browser_mp4(info: &MediaInfo) -> bool {
    matches!(info.video_codec.as_deref(), Some("h264" | "hevc"))
        && matches!(
            info.audio_codec.as_deref(),
            None | Some("aac") | Some("mp3") | Some("opus")
        )
}

fn is_mp4_family_container(container: &str) -> bool {
    container.split(',').any(|part| {
        matches!(
            part.trim(),
            "mp4" | "mov" | "m4a" | "m4v" | "3gp" | "3g2" | "mj2"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{can_remux_to_browser_mp4, MediaInfo};
    use crate::models::media::MediaType;

    #[test]
    fn h264_with_opus_can_be_remuxed_for_browser_playback() {
        let info = MediaInfo {
            media_type: MediaType::Video,
            duration_secs: Some(60.0),
            container: Some("matroska,webm".to_string()),
            video_codec: Some("h264".to_string()),
            audio_codec: Some("opus".to_string()),
            has_video: true,
            has_audio: true,
            video_height: None,
            video_width: None,
        };

        assert!(can_remux_to_browser_mp4(&info));
    }
}
