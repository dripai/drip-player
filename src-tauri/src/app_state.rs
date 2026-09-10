use crate::models::settings::AppSettings;
use crate::services::{
    downloads::DownloadRegistry, persistence::PersistenceManager,
    playback_controller::PlaybackController,
};
use std::sync::{atomic::AtomicU64, Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct AppState {
    pub database: Arc<Mutex<PersistenceManager>>,
    pub settings: Arc<Mutex<AppSettings>>,
    pub playback: Arc<Mutex<PlaybackController>>,
    pub downloads: Arc<Mutex<DownloadRegistry>>,
    pub directory_operation: Arc<Mutex<()>>,
    pub transcription_submission: Arc<tokio::sync::Mutex<()>>,
    pub playlist_snapshot_version: Arc<AtomicU64>,
}

pub fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|error| format!("Application state unavailable: {error}"))
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let database = PersistenceManager::open()?;
        crate::services::downloads::recover(&database)?;
        let settings = database.load_settings()?;
        Ok(Self {
            database: Arc::new(Mutex::new(database)),
            settings: Arc::new(Mutex::new(settings)),
            playback: Arc::new(Mutex::new(PlaybackController::new())),
            downloads: Arc::new(Mutex::new(DownloadRegistry::default())),
            directory_operation: Arc::new(Mutex::new(())),
            transcription_submission: Arc::new(tokio::sync::Mutex::new(())),
            playlist_snapshot_version: Arc::new(AtomicU64::new(0)),
        })
    }
}
