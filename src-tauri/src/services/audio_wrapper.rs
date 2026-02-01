use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver, channel};
use std::thread;
use std::time::Duration;
use crate::services::audio_backend::AudioBackend;

pub enum AudioCommand {
    PlayFile(PathBuf, Sender<Result<Option<Duration>, String>>),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    GetDuration(Sender<Duration>),
}

#[derive(Clone)]
pub struct AudioWrapper {
    tx: Sender<AudioCommand>,
}

impl AudioWrapper {
    pub fn new() -> Self {
        let (tx, rx) = channel::<AudioCommand>();
        
        thread::spawn(move || {
            let mut backend = AudioBackend::new();
            
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    AudioCommand::PlayFile(path, reply_tx) => {
                        let res = backend.play_file(&path);
                        let _ = reply_tx.send(res);
                    },
                    AudioCommand::Pause => backend.pause(),
                    AudioCommand::Resume => backend.resume(),
                    AudioCommand::Stop => backend.stop(),
                    AudioCommand::Seek(dur) => backend.seek(dur),
                    AudioCommand::SetVolume(vol) => backend.set_volume(vol),
                    AudioCommand::GetDuration(reply_tx) => {
                        let d = backend.get_duration();
                        let _ = reply_tx.send(d);
                    }
                }
            }
        });
        
        Self { tx }
    }
    
    pub fn play_file(&self, path: PathBuf) -> Receiver<Result<Option<Duration>, String>> {
        let (tx, rx) = channel();
        let _ = self.tx.send(AudioCommand::PlayFile(path, tx));
        rx
    }
    
    pub fn pause(&self) {
        let _ = self.tx.send(AudioCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(AudioCommand::Resume);
    }
    
    pub fn stop(&self) {
        let _ = self.tx.send(AudioCommand::Stop);
    }
    
    pub fn seek(&self, duration: Duration) {
        let _ = self.tx.send(AudioCommand::Seek(duration));
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = self.tx.send(AudioCommand::SetVolume(volume));
    }

    pub fn get_duration(&self) -> Duration {
        let (tx, rx) = channel();
        if self.tx.send(AudioCommand::GetDuration(tx)).is_ok() {
            rx.recv().unwrap_or(Duration::from_secs(0))
        } else {
            Duration::from_secs(0)
        }
    }
}
