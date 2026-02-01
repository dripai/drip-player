use rodio::{OutputStream, Sink, Decoder, OutputStreamHandle, Source};
use std::fs::File;
use std::io::{BufReader, Read, Cursor};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::{Arc, Mutex, mpsc};
use std::process::{Command, Stdio, Child};
use crate::services::online_resolver::OnlineResolver;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a Command that hides the console window on Windows
#[cfg(windows)]
fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

fn get_duration_with_ffprobe(path: &Path) -> Option<Duration> {
    // First try lib directory
    let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let lib_dir = current_dir.join("lib");
    let ffprobe_in_lib = lib_dir.join("ffprobe.exe");

    let ffprobe_cmd = if ffprobe_in_lib.exists() {
        ffprobe_in_lib.to_string_lossy().to_string()
    } else {
        // Fallback to system PATH
        "ffprobe".to_string()
    };

    println!("Getting duration with ffprobe: {} for {:?}", ffprobe_cmd, path);

    let output = hidden_command(&ffprobe_cmd)
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;

    if output.status.success() {
        let duration_str = String::from_utf8_lossy(&output.stdout);
        let duration_secs: f64 = duration_str.trim().parse().ok()?;
        println!("Duration from ffprobe: {} seconds", duration_secs);
        Some(Duration::from_secs_f64(duration_secs))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("ffprobe failed: {}", stderr);
        None
    }
}

struct FfmpegSource {
    _child: Child,
    reader: Box<dyn Read + Send>,
}

impl Iterator for FfmpegSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = [0u8; 2];
        match self.reader.read_exact(&mut buf) {
            Ok(_) => Some(i16::from_le_bytes(buf)),
            Err(_) => None,
        }
    }
}

impl Source for FfmpegSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        44100
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub struct AudioBackend {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Sink,
    pub duration: Arc<Mutex<Duration>>,
    current_path: Option<PathBuf>,
}

impl AudioBackend {
    pub fn new() -> Self {
        let (stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        Self {
            _stream: stream,
            stream_handle,
            sink,
            duration: Arc::new(Mutex::new(Duration::from_secs(0))),
            current_path: None,
        }
    }

    pub fn play_file(&mut self, path: &Path) -> Result<Option<Duration>, String> {
        self.current_path = Some(path.to_path_buf());

        // Stop current track
        if !self.sink.empty() {
            self.sink.stop();
            // Re-create sink to ensure clean state
            self.sink = Sink::try_new(&self.stream_handle).map_err(|e| e.to_string())?;
        }

        // Try to get duration with ffprobe first (works for all formats)
        let ffprobe_duration = get_duration_with_ffprobe(path);
        if let Some(d) = ffprobe_duration {
            *self.duration.lock().unwrap() = d;
        }

        // Try native rodio decoding first
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);

            // Catch panic from Decoder::new
            let decoder_result = std::panic::catch_unwind(move || {
                Decoder::new(reader)
            });

            match decoder_result {
                Ok(Ok(source)) => {
                    // Use rodio duration if ffprobe didn't work
                    if ffprobe_duration.is_none() {
                        if let Some(d) = source.total_duration() {
                            *self.duration.lock().unwrap() = d;
                        }
                    }
                    self.sink.append(source);
                    self.sink.play();
                    return Ok(ffprobe_duration);
                },
                Ok(Err(e)) => {
                    println!("Error decoding file with rodio {:?}: {}. Trying ffmpeg fallback...", path, e);
                    self.play_with_ffmpeg(path)?;
                },
                Err(e) => {
                    println!("Panic decoding file with rodio {:?}: {:?}. Trying ffmpeg fallback...", path, e);
                    self.play_with_ffmpeg(path)?;
                }
            }
        } else {
            // Probably a URL or unreadable file, try ffmpeg
            self.play_with_ffmpeg(path)?;
        }
        Ok(ffprobe_duration)
    }

    fn play_with_ffmpeg(&mut self, path: &Path) -> Result<(), String> {
        let (_, ffmpeg_path) = OnlineResolver::get_tools_paths();
        
        let ffmpeg_cmd = if let Some(path) = ffmpeg_path {
            let p = Path::new(&path);
            if p.is_dir() {
                p.join("ffmpeg.exe").to_string_lossy().to_string()
            } else {
                path
            }
        } else {
            "ffmpeg".to_string()
        };

        println!("Spawning ffmpeg for playback: {} -i {:?}", ffmpeg_cmd, path);

        let mut child = hidden_command(&ffmpeg_cmd)
            .arg("-i")
            .arg(path)
            .arg("-f")
            .arg("s16le")
            .arg("-ac")
            .arg("2")
            .arg("-ar")
            .arg("44100")
            .arg("-vn")
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("无法播放: 启动 FFmpeg 失败 {}", e))?;

        if let Some(mut stdout) = child.stdout.take() {
            // Implement 5s timeout check for network streams
            let (tx, rx) = mpsc::channel();
            
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096]; // Read a chunk
                match stdout.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let _ = tx.send(Ok((buf[..n].to_vec(), stdout)));
                    },
                    Ok(_) => {
                        let _ = tx.send(Err("EOF immediately".to_string()));
                    },
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            });

            // Wait for initial data or error
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok((initial_data, rest_stdout))) => {
                    // Successfully received data
                    // Create a composite reader: initial data + rest of stdout
                    let reader = Cursor::new(initial_data).chain(rest_stdout);
                    
                    let source = FfmpegSource {
                        _child: child,
                        reader: Box::new(reader),
                    };
                    
                    self.sink.append(source);
                    self.sink.play();
                    Ok(())
                },
                Ok(Err(e)) => {
                    let _ = child.kill();
                    Err(format!("无法播放: {}", e))
                },
                Err(_) => {
                    // Timeout
                    let _ = child.kill();
                    Err("无法播放: 连接超时 (5秒)".to_string())
                }
            }
        } else {
            let _ = child.kill();
            Err("无法播放: 无法获取输出流".to_string())
        }
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn resume(&self) {
        self.sink.play();
    }
    
    pub fn stop(&self) {
        self.sink.stop();
    }
    
    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
    }

    pub fn play_file_from(&mut self, path: &Path, offset: Duration) {
        // Stop current track
        if !self.sink.empty() {
            self.sink.stop();
            self.sink = Sink::try_new(&self.stream_handle).unwrap();
        }

        // Try native rodio decoding first
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            
            let decoder_result = std::panic::catch_unwind(move || {
                Decoder::new(reader)
            });

            match decoder_result {
                Ok(Ok(source)) => {
                    self.sink.append(source.skip_duration(offset));
                    self.sink.play();
                    return;
                },
                Ok(Err(e)) => {
                    println!("Error decoding file with rodio for seek: {}. Trying ffmpeg fallback...", e);
                },
                Err(e) => {
                    println!("Panic decoding file with rodio for seek: {:?}. Trying ffmpeg fallback...", e);
                }
            }
        }
        
        self.play_with_ffmpeg_at(path, offset);
    }

    fn play_with_ffmpeg_at(&mut self, path: &Path, offset: Duration) {
        if let Some(ffmpeg) = OnlineResolver::get_ffmpeg_path() {
            let mut cmd = hidden_command(&ffmpeg);
            
            if offset.as_secs() > 0 {
                cmd.arg("-ss").arg(format!("{}", offset.as_secs_f32()));
            }
            
            let child = cmd
                .arg("-i")
                .arg(path)
                .arg("-f")
                .arg("s16le")
                .arg("-ac")
                .arg("2")
                .arg("-ar")
                .arg("44100")
                .arg("-acodec")
                .arg("pcm_s16le")
                .arg("-")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            match child {
                Ok(mut child) => {
                    if let Some(stdout) = child.stdout.take() {
                        let source = FfmpegSource {
                            _child: child,
                            reader: Box::new(BufReader::new(stdout)),
                        };
                        self.sink.append(source);
                        self.sink.play();
                    }
                },
                Err(e) => println!("Failed to spawn ffmpeg: {}", e),
            }
        }
    }

    pub fn seek(&mut self, time: Duration) {
        if let Some(path) = self.current_path.clone() {
            self.play_file_from(&path, time);
        }
    }
    
    pub fn get_duration(&self) -> Duration {
        *self.duration.lock().unwrap()
    }
}
