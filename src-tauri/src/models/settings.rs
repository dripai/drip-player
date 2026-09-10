use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
pub struct AppSettings {
    pub revision: u64,
    pub theme: String,
    pub language: String,
    pub play_mode: String,
    pub minimize_to_tray: bool,
    pub download_directory: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettingsPatch {
    pub theme: Option<String>,
    pub language: Option<String>,
    pub play_mode: Option<String>,
    pub minimize_to_tray: Option<bool>,
}

impl AppSettings {
    pub fn patched(&self, patch: AppSettingsPatch) -> Result<Self, String> {
        let mut next = self.clone();
        if let Some(theme) = patch.theme {
            if !matches!(theme.as_str(), "auto" | "light" | "dark") {
                return Err("Invalid theme".into());
            }
            next.theme = theme;
        }
        if let Some(language) = patch.language {
            if !matches!(language.as_str(), "zh" | "en") {
                return Err("Invalid interface language".into());
            }
            next.language = language;
        }
        if let Some(mode) = patch.play_mode {
            if !matches!(
                mode.as_str(),
                "sequential" | "random" | "repeat_one" | "repeat_all"
            ) {
                return Err("Invalid play mode".into());
            }
            next.play_mode = mode;
        }
        if let Some(enabled) = patch.minimize_to_tray {
            next.minimize_to_tray = enabled;
        }
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or("Settings revision overflow")?;
        Ok(next)
    }
}
