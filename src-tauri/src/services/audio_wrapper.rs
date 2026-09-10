use crate::models::playback::PlaybackStatus;
use crate::services::audio_backend::AudioBackend;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Sender},
    Arc, Mutex,
};
use std::time::Duration;

pub enum AudioControl {
    Pause,
    Resume,
    Seek(f64),
    Volume(f32),
}
enum AudioCommand {
    Load(u64, PathBuf),
    Control(u64, AudioControl, Sender<Result<(), String>>),
    Stop,
}

#[derive(Clone)]
pub struct AudioSnapshot {
    pub session_id: u64,
    pub status: PlaybackStatus,
    pub position: f64,
    pub duration: f64,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct AudioWrapper {
    tx: Sender<AudioCommand>,
    desired: Arc<AtomicU64>,
    snapshot: Arc<Mutex<Option<AudioSnapshot>>>,
}
impl AudioWrapper {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let desired = Arc::new(AtomicU64::new(0));
        let snapshot = Arc::new(Mutex::new(None));
        let target = desired.clone();
        let shared = snapshot.clone();
        std::thread::spawn(move || {
            let mut backend: Option<AudioBackend> = None;
            let mut loaded = 0;
            let mut failed = false;
            loop {
                let command = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => Some(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let mut result: Result<(), String> = Ok(());
                let mut completion = None;
                match command {
                    Some(AudioCommand::Load(id, path)) if target.load(Ordering::SeqCst) == id => {
                        loaded = id;
                        failed = false;
                        result = (|| {
                            if backend.is_none() {
                                backend = Some(AudioBackend::new()?);
                            }
                            let audio = backend.as_mut().ok_or("Audio output unavailable")?;
                            audio.load(path)?;
                            if target.load(Ordering::SeqCst) == id {
                                audio.resume();
                            } else {
                                audio.stop();
                            }
                            Ok(())
                        })();
                    }
                    Some(AudioCommand::Control(id, control, reply)) => {
                        if id != loaded || target.load(Ordering::SeqCst) != id || failed {
                            let _ = reply.send(Err("Audio session is no longer active".into()));
                            continue;
                        }
                        completion = Some(reply);
                        if let Some(audio) = backend.as_mut() {
                            match control {
                                AudioControl::Pause => audio.pause(),
                                AudioControl::Resume => audio.resume(),
                                AudioControl::Volume(volume) => audio.set_volume(volume),
                                AudioControl::Seek(position) => result = audio.seek(position),
                            }
                        } else {
                            result = Err("Audio output unavailable".into());
                        }
                    }
                    Some(AudioCommand::Stop) => {
                        if let Some(audio) = &backend {
                            audio.stop();
                        }
                        loaded = 0;
                    }
                    _ => {}
                }
                if loaded == 0 {
                    continue;
                }
                if target.load(Ordering::SeqCst) != loaded {
                    if let Some(audio) = &backend {
                        audio.stop();
                    }
                    if let Some(reply) = completion {
                        let _ = reply.send(Err("Audio session changed".into()));
                    }
                    continue;
                }
                let acknowledged = result.clone();
                if let Err(error) = result {
                    failed = true;
                    if let Some(audio) = &backend {
                        audio.stop();
                    }
                    *shared.lock().unwrap() = Some(AudioSnapshot {
                        session_id: loaded,
                        status: PlaybackStatus::Failed,
                        position: 0.0,
                        duration: 0.0,
                        error: Some(error),
                    });
                } else if !failed {
                    if let Some(audio) = &backend {
                        *shared.lock().unwrap() = Some(AudioSnapshot {
                            session_id: loaded,
                            status: if audio.ended() {
                                PlaybackStatus::Ended
                            } else if audio.paused() {
                                PlaybackStatus::Paused
                            } else {
                                PlaybackStatus::Playing
                            },
                            position: audio.position(),
                            duration: audio.duration(),
                            error: None,
                        });
                    }
                }
                if let Some(reply) = completion {
                    let _ = reply.send(acknowledged);
                }
            }
        });
        Self {
            tx,
            desired,
            snapshot,
        }
    }
    pub fn load(&self, id: u64, path: PathBuf) -> Result<(), String> {
        self.desired.store(id, Ordering::SeqCst);
        self.tx
            .send(AudioCommand::Load(id, path))
            .map_err(|error| error.to_string())
    }
    pub fn stop(&self) -> Result<(), String> {
        self.desired.store(0, Ordering::SeqCst);
        self.tx
            .send(AudioCommand::Stop)
            .map_err(|error| error.to_string())
    }
    pub fn control(&self, id: u64, control: AudioControl) -> Result<(), String> {
        let (reply, received) = mpsc::channel();
        self.tx
            .send(AudioCommand::Control(id, control, reply))
            .map_err(|error| error.to_string())?;
        received
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("Audio control was not acknowledged: {error}"))?
    }
    pub fn snapshot(&self) -> Option<AudioSnapshot> {
        self.snapshot.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::toolchain;
    use std::time::Instant;
    #[test]
    #[ignore = "requires bundled FFmpeg and an audio output device; the fixture is silent"]
    fn audio_controls_acknowledge_the_updated_snapshot() {
        toolchain::set_resource_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("silent.wav");
        let result = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=16000:cl=mono",
                "-t",
                "1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(result.status.success());
        let audio = AudioWrapper::new();
        audio.load(1, path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while audio.snapshot().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        audio.control(1, AudioControl::Pause).unwrap();
        assert_eq!(audio.snapshot().unwrap().status, PlaybackStatus::Paused);
        audio.control(1, AudioControl::Seek(0.7)).unwrap();
        assert!((audio.snapshot().unwrap().position - 0.7).abs() < 0.05);
        audio.control(1, AudioControl::Resume).unwrap();
        assert_eq!(audio.snapshot().unwrap().status, PlaybackStatus::Playing);
        let deadline = Instant::now() + Duration::from_secs(3);
        while audio.snapshot().unwrap().status != PlaybackStatus::Ended && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(audio.snapshot().unwrap().status, PlaybackStatus::Ended);
        audio.control(1, AudioControl::Seek(0.2)).unwrap();
        assert!((audio.snapshot().unwrap().position - 0.2).abs() < 0.05);
        assert!(audio.control(2, AudioControl::Pause).is_err());
        audio.stop().unwrap();
    }
}
