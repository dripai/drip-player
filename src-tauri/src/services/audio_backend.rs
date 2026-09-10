use crate::services::{media_probe, toolchain};
use rodio::{
    ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;

struct FfmpegSource {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}
impl Iterator for FfmpegSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let mut sample = [0; 4];
        self.reader.read_exact(&mut sample).ok()?;
        Some(f32::from_le_bytes(sample))
    }
}
impl Source for FfmpegSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        rodio::math::nz!(2)
    }
    fn sample_rate(&self) -> SampleRate {
        rodio::math::nz!(44100)
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
impl Drop for FfmpegSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct AudioBackend {
    player: Player,
    output: MixerDeviceSink,
    position_offset: f64,
    duration: f64,
    path: Option<PathBuf>,
    volume: f32,
}
impl AudioBackend {
    pub fn new() -> Result<Self, String> {
        let output = DeviceSinkBuilder::from_default_device()
            .and_then(|builder| builder.open_stream())
            .map_err(|error| format!("Audio output unavailable: {error}"))?;
        let player = Player::connect_new(output.mixer());
        Ok(Self {
            player,
            output,
            position_offset: 0.0,
            duration: 0.0,
            path: None,
            volume: 1.0,
        })
    }
    pub fn load(&mut self, path: PathBuf) -> Result<(), String> {
        self.stop();
        self.duration = media_probe::duration(&path)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        self.path = Some(path.clone());
        self.load_at(&path, Duration::ZERO)
    }
    fn load_at(&mut self, path: &Path, offset: Duration) -> Result<(), String> {
        self.player.stop();
        self.player = Player::connect_new(self.output.mixer());
        self.player.pause();
        self.player.set_volume(self.volume);
        self.position_offset = offset.as_secs_f64();
        let file = File::open(path)
            .map_err(|error| format!("Cannot open audio {}: {error}", path.display()))?;
        let decoded = std::panic::catch_unwind(|| Decoder::try_from(file));
        if let Ok(Ok(source)) = decoded {
            if self.duration == 0.0 {
                self.duration = source
                    .total_duration()
                    .map(|value| value.as_secs_f64())
                    .unwrap_or(0.0);
            }
            self.player.append(source.skip_duration(offset));
        } else {
            // Preserve the existing FFmpeg decoder for formats rodio cannot decode.
            let mut child = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
                .args(["-v", "error", "-ss"])
                .arg(offset.as_secs_f64().to_string())
                .arg("-i")
                .arg(path)
                .args(["-f", "f32le", "-ac", "2", "-ar", "44100", "-vn", "-"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            let stdout = child.stdout.take().ok_or("FFmpeg has no audio output")?;
            let mut reader = BufReader::new(stdout);
            match reader.fill_buf() {
                Ok(bytes) if !bytes.is_empty() => {}
                Ok(_) => {
                    let status = child.wait().map_err(|error| error.to_string())?;
                    return Err(format!("FFmpeg produced no audio: {status}"));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Cannot read FFmpeg audio: {error}"));
                }
            }
            self.player.append(FfmpegSource { child, reader });
        }
        Ok(())
    }
    pub fn seek(&mut self, position: f64) -> Result<(), String> {
        let path = self.path.clone().ok_or("No audio loaded")?;
        let paused = self.player.is_paused();
        self.load_at(&path, Duration::from_secs_f64(position))?;
        if !paused {
            self.player.play();
        }
        Ok(())
    }
    pub fn pause(&self) {
        self.player.pause();
    }
    pub fn resume(&self) {
        self.player.play();
    }
    pub fn stop(&self) {
        self.player.stop();
    }
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.player.set_volume(volume);
    }
    pub fn position(&self) -> f64 {
        // Rodio tracks consumed audio; device buffering can lead actual speaker output.
        self.position_offset + self.player.get_pos().as_secs_f64()
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
    pub fn ended(&self) -> bool {
        self.player.empty()
    }
    pub fn paused(&self) -> bool {
        self.player.is_paused()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Instant;

    #[test]
    #[ignore = "requires the bundled FFmpeg tools and an audio output device; playback is muted"]
    fn muted_audio_play_pause_seek_and_complete() {
        toolchain::set_resource_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let dir = tempfile::tempdir().unwrap();
        let mut audio = AudioBackend::new().expect("audio output device");
        audio.set_volume(0.0);
        for extension in ["wav", "opus"] {
            let path = dir.path().join(format!("silent.{extension}"));
            let result = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=44100:cl=stereo",
                    "-t",
                    "2",
                ])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            audio.load(path).unwrap();
            assert!(audio.paused());
            assert!((audio.duration() - 2.0).abs() < 0.1);
            audio.resume();
            let deadline = Instant::now() + Duration::from_secs(3);
            while audio.position() < 0.1 && Instant::now() < deadline {
                sleep(Duration::from_millis(10));
            }
            assert!(audio.position() >= 0.1, "{extension}: no playback progress");
            audio.pause();
            sleep(Duration::from_millis(100));
            let paused_position = audio.position();
            sleep(Duration::from_millis(100));
            assert!(
                (audio.position() - paused_position).abs() < 0.03,
                "{extension}: advanced while paused"
            );
            audio.seek(1.0).unwrap();
            assert!(audio.paused());
            assert!((audio.position() - 1.0).abs() < 0.03);
            audio.resume();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !audio.ended() && Instant::now() < deadline {
                sleep(Duration::from_millis(10));
            }
            assert!(audio.ended(), "{extension}: did not finish");
            assert!(audio.position() > 1.8, "{extension}: lost final position");
        }
    }
}
