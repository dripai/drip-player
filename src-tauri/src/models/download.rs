use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadPhase {
    Resolving,
    Downloading,
    Publishing,
    Completed,
    Failed,
    Canceled,
    Interrupted,
    RecoveryRequired,
}

impl DownloadPhase {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Resolving | Self::Downloading | Self::Publishing)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub percent: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadJob {
    pub id: String,
    pub url: String,
    pub title: String,
    pub media_id: Option<String>,
    pub attempt: u64,
    pub phase: DownloadPhase,
    // History of the last attempt, never the source of a retry's destination.
    pub directory: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub interrupt_reason: Option<String>,
    pub created_at: u64,
    #[serde(skip_deserializing)]
    pub progress: Option<DownloadProgress>,
}

impl DownloadJob {
    pub fn new(url: String) -> Result<Self, String> {
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: url.clone(),
            url,
            media_id: None,
            attempt: 0,
            phase: DownloadPhase::Interrupted,
            directory: None,
            output_path: None,
            error: None,
            interrupt_reason: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| format!("无法读取下载任务时间：{error}"))?
                .as_millis()
                .try_into()
                .map_err(|_| "Download timestamp overflow")?,
            progress: None,
        })
    }
}

#[derive(Serialize)]
pub struct DownloadSnapshot {
    pub revision: u64,
    pub jobs: Vec<DownloadJob>,
}
