use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningSettings {
    pub revision: u64,
    pub workspace_id: String,
    pub text_model: String,
    pub transcription_model: String,
    pub evaluation_app_id: String,
    pub subtitle_display: String,
    pub translation_language: String,
    pub subtitle_font_size: u16,
    pub auto_scroll: bool,
    pub playback_rate: f64,
    pub repetitions: u8,
    pub gap_seconds: f64,
    pub skip_silence: bool,
    pub recording_device: String,
    pub recording_directory: String,
    pub playback_order: String,
}

impl Default for LearningSettings {
    fn default() -> Self {
        Self {
            revision: 0,
            workspace_id: String::new(),
            text_model: "qwen-plus".into(),
            transcription_model: "paraformer-v2".into(),
            evaluation_app_id: String::new(),
            subtitle_display: "bilingual".into(),
            translation_language: "zh".into(),
            subtitle_font_size: 18,
            auto_scroll: true,
            playback_rate: 1.0,
            repetitions: 2,
            gap_seconds: 1.0,
            skip_silence: false,
            recording_device: "default".into(),
            recording_directory: String::new(),
            playback_order: "original_first".into(),
        }
    }
}
impl LearningSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self
            .workspace_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || self.workspace_id.len() > 128
        {
            return Err("业务空间 ID 格式不正确".into());
        }
        if self.text_model.trim().is_empty()
            || self.text_model.len() > 128
            || self.transcription_model != "paraformer-v2"
        {
            return Err("模型配置不正确".into());
        }
        if !matches!(self.subtitle_display.as_str(), "original" | "bilingual")
            || !matches!(
                self.translation_language.as_str(),
                "zh" | "en" | "ja" | "ko"
            )
            || !(12..=36).contains(&self.subtitle_font_size)
            || !self.playback_rate.is_finite()
            || !(0.5..=2.0).contains(&self.playback_rate)
            || !(1..=10).contains(&self.repetitions)
            || !self.gap_seconds.is_finite()
            || !(0.0..=10.0).contains(&self.gap_seconds)
            || !matches!(
                self.playback_order.as_str(),
                "original_first" | "recording_first"
            )
            || self.recording_device.is_empty()
        {
            return Err("学习参数超出允许范围".into());
        }
        if !self.recording_directory.is_empty()
            && !std::path::Path::new(&self.recording_directory).is_dir()
        {
            return Err("录音保存目录不存在".into());
        }
        Ok(())
    }
    pub fn endpoint(&self) -> Result<String, String> {
        if self.workspace_id.is_empty() {
            return Err("请先在设置中填写百炼北京地域的业务空间 ID".into());
        }
        self.validate()?;
        Ok(format!(
            "https://{}.cn-beijing.maas.aliyuncs.com",
            self.workspace_id
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Cue {
    pub id: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub translation: Option<String>,
    pub translation_language: Option<String>,
    pub favorite: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub media_id: String,
    pub label: String,
    pub cues: Vec<Cue>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub transcript_id: String,
    pub cue_id: usize,
    pub path: String,
    pub created_at: u64,
    pub evaluation: Option<String>,
}

#[derive(Serialize)]
pub struct LearningTextResult {
    pub text: String,
    pub language: String,
}
