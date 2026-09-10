use crate::models::download::DownloadOption;
use crate::models::media::MediaType;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RemoteFormat {
    pub format_id: String,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub ext: Option<String>,
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub duration: Option<f64>,
    pub has_drm: Option<bool>,
}

impl RemoteFormat {
    fn video_codec_name(&self) -> Option<String> {
        let codec = self.vcodec.as_deref()?.to_lowercase();
        if codec.is_empty() || codec == "none" {
            return None;
        }
        Some(
            if codec.starts_with("avc1") || codec.starts_with("avc3") || codec == "h264" {
                "H.264"
            } else if codec.starts_with("hev1")
                || codec.starts_with("hvc1")
                || matches!(codec.as_str(), "hevc" | "h265")
            {
                "HEVC"
            } else if codec.starts_with("av01") || codec == "av1" {
                "AV1"
            } else if codec.starts_with("vp09") || codec == "vp9" {
                "VP9"
            } else {
                return Some(codec.to_uppercase());
            }
            .into(),
        )
    }

    fn has_aac(&self) -> bool {
        self.acodec
            .as_deref()
            .is_some_and(|codec| codec == "aac" || codec.starts_with("mp4a"))
    }

    fn has_video(&self) -> bool {
        self.vcodec.as_deref() != Some("none")
            && (self
                .vcodec
                .as_deref()
                .is_some_and(|codec| !codec.is_empty())
                || self.height.is_some_and(|height| height > 0)
                || self.width.is_some_and(|width| width > 0)
                || self.ext.as_deref().is_some_and(|ext| {
                    crate::services::media_capabilities::is_video_path(std::path::Path::new(
                        &format!("media.{ext}"),
                    ))
                }))
    }

    fn audio_only(&self) -> bool {
        !self.has_video()
            && self.acodec.as_deref() != Some("none")
            && (self.vcodec.as_deref() == Some("none")
                || self.ext.as_deref().is_some_and(|ext| {
                    crate::services::media_capabilities::is_audio_path(std::path::Path::new(
                        &format!("media.{ext}"),
                    ))
                }))
    }

    fn selector(&self) -> Result<String, String> {
        if self.format_id.is_empty()
            || !self
                .format_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err("站点返回了无法选择的格式标识".into());
        }
        Ok(self.format_id.clone())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct VideoMetadata {
    pub title: String,
    pub id: String,
    pub season: Option<String>,
    pub series: Option<String>,
    pub duration: Option<f64>,
    pub is_live: Option<bool>,
    #[serde(default)]
    pub formats: Vec<RemoteFormat>,
}

pub fn duration_tolerance(expected: f64) -> f64 {
    (expected * 0.02).clamp(2.0, 30.0)
}

impl VideoMetadata {
    pub fn display_title(&self) -> String {
        let mut parts = Vec::new();
        for part in [
            self.series.as_deref(),
            self.season.as_deref(),
            Some(self.title.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            let part = part.trim();
            if !part.is_empty() && !parts.contains(&part) {
                parts.push(part);
            }
        }
        parts.join(" - ")
    }

    pub fn expected_duration(&self) -> Option<f64> {
        self.duration
            .filter(|value| value.is_finite() && *value > 0.0)
    }

    pub fn download_options(&self) -> Result<Vec<DownloadOption>, String> {
        if self.is_live == Some(true) {
            return Err("当前是直播链接，请使用已结束的视频链接".into());
        }
        // Keep the selected resolution. Within it, prefer complete H.264/AAC
        // streams; yt-dlp's preference breaks ties, not browser compatibility.
        let formats: Vec<_> = self
            .formats
            .iter()
            .filter(|format| format.has_drm != Some(true))
            .collect();
        let audio = formats.iter().copied().rfind(|format| format.audio_only());
        let video_audio = formats
            .iter()
            .copied()
            .filter(|format| format.audio_only())
            .max_by_key(|format| {
                (
                    self.limited_duration(format.duration).is_none(),
                    format.has_aac(),
                )
            });
        let mut videos = BTreeMap::new();
        for format in formats.iter().copied().filter(|format| format.has_video()) {
            let height = format.height.filter(|height| *height > 0);
            let mut selector = format.selector()?;
            let companion = if format.acodec.as_deref() == Some("none") {
                video_audio
            } else {
                None
            };
            if let Some(audio) = companion {
                selector.push('+');
                selector.push_str(&audio.selector()?);
            }
            let limited_duration = self
                .limited_duration(format.duration)
                .or_else(|| companion.and_then(|audio| self.limited_duration(audio.duration)));
            let video_codec = format.video_codec_name();
            let rank = (
                limited_duration.is_none(),
                video_codec.as_deref() == Some("H.264"),
                companion.unwrap_or(format).has_aac(),
            );
            let option = DownloadOption {
                id: height
                    .map(|height| format!("video:{height}"))
                    .unwrap_or_else(|| "video:original".into()),
                media_type: MediaType::Video,
                height,
                width: format.width.filter(|width| *width > 0),
                format_selector: selector,
                extract_audio: false,
                requires_audio: companion.is_some()
                    || format
                        .acodec
                        .as_deref()
                        .is_some_and(|codec| codec != "none" && !codec.is_empty()),
                limited_duration,
                video_codec,
            };
            if videos
                .get(&height)
                .is_none_or(|(previous, _)| rank >= *previous)
            {
                videos.insert(height, (rank, option));
            }
        }
        let mut options: Vec<_> = videos
            .into_values()
            .rev()
            .map(|(_, option)| option)
            .collect();
        if options.is_empty() {
            if let Some(audio) = audio {
                options.push(DownloadOption {
                    id: "audio:original".into(),
                    media_type: MediaType::Audio,
                    height: None,
                    width: None,
                    format_selector: audio.selector()?,
                    extract_audio: false,
                    requires_audio: true,
                    limited_duration: self.limited_duration(audio.duration),
                    video_codec: None,
                });
            }
        }
        let audio_source = audio.or_else(|| {
            formats
                .iter()
                .copied()
                .rfind(|format| format.has_video() && format.acodec.as_deref() != Some("none"))
        });
        if let Some(audio) = audio_source {
            options.push(DownloadOption {
                id: "audio:mp3".into(),
                media_type: MediaType::Audio,
                height: None,
                width: None,
                format_selector: audio.selector()?,
                extract_audio: true,
                requires_audio: true,
                limited_duration: self.limited_duration(audio.duration),
                video_codec: None,
            });
        }
        if options.is_empty() {
            return Err("没有可下载的音视频格式；缺少类型信息或当前账号没有可用格式".into());
        }
        Ok(options)
    }

    fn limited_duration(&self, actual: Option<f64>) -> Option<f64> {
        self.expected_duration()
            .zip(actual.filter(|value| value.is_finite() && *value > 0.0))
            .and_then(|(expected, actual)| {
                (expected - actual > duration_tolerance(expected)).then_some(actual)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilibili_null_codecs_keep_video_and_film_context() {
        let metadata: VideoMetadata = serde_json::from_value(serde_json::json!({
            "id":"747309", "title":"正片", "series":"理查德·柯蒂斯三部曲", "season":"时空恋旅人", "duration":7403,
            "formats":[{"format_id":"32","ext":"mp4","height":480,"vcodec":null,"acodec":null}]
        })).unwrap();
        assert_eq!(
            metadata.display_title(),
            "理查德·柯蒂斯三部曲 - 时空恋旅人 - 正片"
        );
        let options = metadata.download_options().unwrap();
        assert_eq!(options[0].id, "video:480");
        assert_eq!(options[0].format_selector, "32");
        assert!(!options[0].extract_audio);
        assert_eq!(options[1].id, "audio:mp3");
    }

    #[test]
    fn resolutions_use_available_formats_and_merge_separate_audio() {
        let metadata: VideoMetadata = serde_json::from_value(serde_json::json!({
            "id":"clip", "title":"Clip", "duration":60,
            "formats":[
                {"format_id":"a","vcodec":"none","acodec":"aac","ext":"m4a"},
                {"format_id":"v480","height":480,"vcodec":"avc1","acodec":"none"},
                {"format_id":"v720old","height":720,"vcodec":"avc1","acodec":"none"},
                {"format_id":"v720","height":720,"vcodec":"avc1","acodec":"none"},
                {"format_id":"drm","height":1080,"vcodec":"avc1","has_drm":true}
            ]
        }))
        .unwrap();
        let options = metadata.download_options().unwrap();
        assert_eq!(
            options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["video:720", "video:480", "audio:mp3"]
        );
        assert_eq!(options[0].format_selector, "v720+a");
        assert!(options[0].requires_audio);
    }

    #[test]
    fn audio_keeps_original_format_and_known_previews_are_marked() {
        let metadata: VideoMetadata = serde_json::from_value(serde_json::json!({
            "id":"audio", "title":"Audio", "duration":7403,
            "formats":[{"format_id":"a", "ext":"wav", "duration":360}]
        }))
        .unwrap();
        let options = metadata.download_options().unwrap();
        assert_eq!(options[0].id, "audio:original");
        assert!(!options[0].extract_audio);
        assert_eq!(options[0].limited_duration, Some(360.0));
        let unknown: VideoMetadata = serde_json::from_value(
            serde_json::json!({"id":"x","title":"x","formats":[{"format_id":"x"}]}),
        )
        .unwrap();
        assert!(unknown.download_options().is_err());
    }

    #[test]
    fn same_resolution_prefers_complete_h264_and_aac_without_lowering_quality() {
        let metadata: VideoMetadata = serde_json::from_value(serde_json::json!({
            "id":"film", "title":"Film", "duration":5000,
            "formats":[
                {"format_id":"30280","vcodec":"none","acodec":"mp4a.40.2","ext":"m4a"},
                {"format_id":"30251","vcodec":"none","acodec":"flac","ext":"flac"},
                {"format_id":"avc720","height":720,"vcodec":"avc1.64001f","acodec":"none"},
                {"format_id":"avc1080","height":1080,"vcodec":"avc1.640028","acodec":"none"},
                {"format_id":"30077","height":1080,"vcodec":"hev1.1.6.L120","acodec":"none"},
                {"format_id":"hevc2160","height":2160,"vcodec":"hvc1.1.6.L150","acodec":"none"},
                {"format_id":"avc2160preview","height":2160,"duration":360,"vcodec":"avc1.640033","acodec":"none"}
            ]
        })).unwrap();
        let options = metadata.download_options().unwrap();
        assert_eq!(options[0].id, "video:2160");
        assert_eq!(options[0].format_selector, "hevc2160+30280");
        assert_eq!(options[0].video_codec.as_deref(), Some("HEVC"));
        assert!(options[0].limited_duration.is_none());
        assert_eq!(options[1].id, "video:1080");
        assert_eq!(options[1].format_selector, "avc1080+30280");
        assert_eq!(options[1].video_codec.as_deref(), Some("H.264"));
        assert_eq!(options[2].format_selector, "avc720+30280");
        // Explicit audio-only downloads still keep the best original source.
        assert_eq!(options[3].format_selector, "30251");
    }
}
