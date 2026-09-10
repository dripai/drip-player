use crate::models::download::{DownloadAuth, DownloadOption};
use crate::services::download_options::VideoMetadata;
use crate::services::download_process::{run_command, DownloadControl};
use crate::services::toolchain;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Supported video platforms with their specific configurations
#[derive(Debug, Clone, PartialEq)]
pub enum VideoPlatform {
    Bilibili,
    YouTube,
    Douyin,       // 抖音
    TencentVideo, // 腾讯视频
    Weixin,       // 微信视频号
    Generic,      // 通用/其他平台
}

impl VideoPlatform {
    /// Detect platform from URL
    pub fn from_url(url: &str) -> Self {
        let url_lower = url.to_lowercase();

        if url_lower.contains("bilibili.com") || url_lower.contains("b23.tv") {
            VideoPlatform::Bilibili
        } else if url_lower.contains("youtube.com") || url_lower.contains("youtu.be") {
            VideoPlatform::YouTube
        } else if url_lower.contains("douyin.com") || url_lower.contains("iesdouyin.com") {
            VideoPlatform::Douyin
        } else if url_lower.contains("v.qq.com") || url_lower.contains("qq.com/x/cover") {
            VideoPlatform::TencentVideo
        } else if url_lower.contains("channels.weixin.qq.com")
            || url_lower.contains("finder.video.qq.com")
        {
            VideoPlatform::Weixin
        } else {
            VideoPlatform::Generic
        }
    }

    /// Get the referer header for this platform (if needed)
    pub fn get_referer(&self) -> Option<&'static str> {
        match self {
            VideoPlatform::Bilibili => Some("https://www.bilibili.com/"),
            VideoPlatform::Douyin => Some("https://www.douyin.com/"),
            VideoPlatform::TencentVideo => Some("https://v.qq.com/"),
            VideoPlatform::Weixin => Some("https://channels.weixin.qq.com/"),
            VideoPlatform::YouTube => None, // YouTube doesn't need referer
            VideoPlatform::Generic => None,
        }
    }

    /// Get platform display name
    pub fn display_name(&self) -> &'static str {
        match self {
            VideoPlatform::Bilibili => "哔哩哔哩",
            VideoPlatform::YouTube => "YouTube",
            VideoPlatform::Douyin => "抖音",
            VideoPlatform::TencentVideo => "腾讯视频",
            VideoPlatform::Weixin => "微信视频号",
            VideoPlatform::Generic => "通用",
        }
    }

    /// Get the login URL for this platform
    pub fn get_login_url(&self) -> &'static str {
        match self {
            VideoPlatform::Bilibili => "https://passport.bilibili.com/login",
            VideoPlatform::YouTube => "https://accounts.google.com/ServiceLogin?service=youtube",
            VideoPlatform::Douyin => "https://www.douyin.com/login",
            VideoPlatform::TencentVideo => "https://v.qq.com/",
            VideoPlatform::Weixin => "https://channels.weixin.qq.com/",
            VideoPlatform::Generic => "",
        }
    }

    /// Check if URL matches this platform's CDN/stream domains
    pub fn matches_stream_url(url: &str) -> Option<Self> {
        let url_lower = url.to_lowercase();

        if url_lower.contains("bilivideo") || url_lower.contains("bilibili") {
            Some(VideoPlatform::Bilibili)
        } else if url_lower.contains("googlevideo.com") || url_lower.contains("youtube") {
            Some(VideoPlatform::YouTube)
        } else if url_lower.contains("douyinvod")
            || url_lower.contains("bytedance")
            || url_lower.contains("amemv")
        {
            Some(VideoPlatform::Douyin)
        } else if url_lower.contains("v.qq.com") || url_lower.contains("gtimg.com") {
            Some(VideoPlatform::TencentVideo)
        } else {
            None
        }
    }
}

pub struct OnlineResolver;

pub struct DownloadRequest<'a> {
    pub url: &'a str,
    pub title: &'a str,
    pub output_dir: &'a Path,
    pub option: &'a DownloadOption,
    pub auth: DownloadAuth,
    pub extra_subtitle_lang: Option<&'a str>,
}

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

fn configure_source(cmd: &mut Command, platform: &VideoPlatform, auth: DownloadAuth) {
    cmd.args(["--user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"]);
    if let Some(referer) = platform.get_referer() {
        cmd.arg("--referer").arg(referer);
    }
    if let Some(browser) = auth.browser() {
        cmd.arg("--cookies-from-browser").arg(browser);
    }
}

impl OnlineResolver {
    /// Sanitize a string to be used as a filesystem-safe filename.
    pub fn sanitize_filename(name: &str) -> String {
        fn smart_truncate(s: &str, max_len: usize) -> String {
            if s.len() <= max_len {
                return s.to_string();
            }
            if max_len <= 4 {
                return s.chars().take(max_len).collect();
            }
            let take = (max_len - 1) / 2;
            let end_take = max_len - 1 - take;
            let start: String = s.chars().take(take).collect();
            let end: String = s
                .chars()
                .rev()
                .take(end_take)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            format!("{}…{}", start, end)
        }

        let mut s = name.trim().to_string();
        // Replace characters not allowed in filenames on Windows and other platforms
        for ch in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
            s = s.replace(ch, "-");
        }
        // Remove control characters
        s = s.chars().filter(|c| !c.is_control()).collect();
        // Collapse sequences of whitespace
        let mut out = String::with_capacity(s.len());
        let mut last_space = false;
        for ch in s.chars() {
            if ch.is_whitespace() {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            } else {
                out.push(ch);
                last_space = false;
            }
        }
        let mut out = out.trim().to_string();
        // Smart truncate to a reasonable length (100 chars)
        out = smart_truncate(&out, 100);
        // Windows filenames can't end with space or dot
        while out.ends_with(' ') || out.ends_with('.') {
            out.pop();
        }
        if out.is_empty() {
            "untitled".to_string()
        } else {
            out
        }
    }
    pub fn get_tools_paths() -> (String, Option<String>) {
        let yt_dlp_cmd = toolchain::tool_path("yt-dlp");
        let ffmpeg_cmd =
            toolchain::tool_dir_for("ffmpeg").map(|dir| dir.to_string_lossy().to_string());
        (yt_dlp_cmd, ffmpeg_cmd)
    }

    pub fn get_ffplay_path() -> Option<String> {
        toolchain::find_tool("ffplay").map(|path| path.to_string_lossy().to_string())
    }

    pub fn get_mpv_path() -> Option<String> {
        toolchain::find_tool("mpv").map(|path| path.to_string_lossy().to_string())
    }

    pub fn get_ffmpeg_path() -> Option<String> {
        toolchain::find_tool("ffmpeg").map(|path| path.to_string_lossy().to_string())
    }

    pub fn resolve_metadata(
        url: &str,
        auth: DownloadAuth,
        control: &DownloadControl,
    ) -> Result<VideoMetadata, String> {
        let (yt_dlp, _) = Self::get_tools_paths();
        let platform = VideoPlatform::from_url(url);
        let mut cmd = hidden_command(&yt_dlp);
        cmd.args([
            "--ignore-config",
            "--encoding",
            "utf-8",
            "--dump-json",
            "--no-playlist",
            "--no-warnings",
        ]);
        configure_source(&mut cmd, &platform, auth);
        cmd.arg(url);
        let output = run_command(cmd, control, true, |_| Ok(()))?;
        if !output.status.success() {
            let lower = output.stderr.to_lowercase();
            if !platform.get_login_url().is_empty()
                && ["sign in", "login", "log in"]
                    .iter()
                    .any(|needle| lower.contains(needle))
            {
                return Err(format!(
                    "LOGIN_REQUIRED:{}:{}:{}",
                    platform.display_name(),
                    platform.get_login_url(),
                    output.stderr.trim()
                ));
            }
            return Err(format!("yt-dlp: {}", output.stderr.trim()));
        }
        let mut entries = output.stdout.lines().filter(|line| !line.trim().is_empty());
        let metadata = serde_json::from_str(entries.next().ok_or("No media metadata returned")?)
            .map_err(|error| format!("Invalid media metadata: {error}"))?;
        if entries.next().is_some() {
            return Err("The URL contains multiple episodes; use a single episode URL".into());
        }
        Ok(metadata)
    }

    /// Try to resolve metadata using OAuth2 authentication
    /// This will open a browser for user to authorize
    /// Returns Ok(metadata) if successful, Err with message if failed
    pub fn resolve_metadata_with_oauth(url: &str) -> Result<VideoMetadata, String> {
        let (yt_dlp_cmd, _) = Self::get_tools_paths();
        let platform = VideoPlatform::from_url(url);

        // OAuth2 is primarily for YouTube
        if platform != VideoPlatform::YouTube {
            return Err(format!(
                "OAuth2 is only supported for YouTube, not {}",
                platform.display_name()
            ));
        }

        println!("Attempting OAuth2 authentication for YouTube...");

        // First, run yt-dlp with --username oauth2 to trigger OAuth flow
        // This will open browser for authorization
        let mut cmd = hidden_command(&yt_dlp_cmd);
        cmd.arg("--dump-json")
           .arg("--no-playlist")
           .arg("--no-warnings")
           .arg("--username").arg("oauth2")
           .arg("--password").arg("")  // Empty password for OAuth
           .arg("--user-agent")
           .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
           .arg("--extractor-args")
           .arg("youtube:player_client=web,default")
           .arg(url);

        println!("Running yt-dlp with OAuth2...");

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Ok(metadata) = serde_json::from_str::<VideoMetadata>(line) {
                    println!(
                        "Successfully resolved metadata with OAuth2: {}",
                        metadata.title
                    );
                    return Ok(metadata);
                }
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!(
                "OAuth2 succeeded but failed to parse response. Stderr: {}",
                stderr
            ))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("OAuth2 failed: {}", stderr);
            Err(format!("OAuth2 authentication failed: {}", stderr))
        }
    }

    /// Parse login required error to get platform info
    pub fn parse_login_error(error: &str) -> Option<(String, String, String)> {
        if error.starts_with("LOGIN_REQUIRED:") {
            let parts: Vec<&str> = error.splitn(4, ':').collect();
            if parts.len() >= 4 {
                return Some((
                    parts[1].to_string(), // platform name
                    parts[2].to_string(), // login url
                    parts[3].to_string(), // original error
                ));
            }
        }
        None
    }

    pub fn download_media(
        request: &DownloadRequest<'_>,
        control: &DownloadControl,
        on_progress: impl Fn(&str) -> Result<(), String>,
    ) -> Result<PathBuf, String> {
        let platform = VideoPlatform::from_url(request.url);
        let DownloadRequest {
            url,
            title,
            output_dir,
            option,
            auth,
            extra_subtitle_lang,
        } = *request;
        let safe_title = Self::sanitize_filename(title).replace('%', "%%");
        let output_template = output_dir.join(format!("{}.%(ext)s", safe_title));
        let (yt_dlp_cmd, ffmpeg_dir) = Self::get_tools_paths();
        let ffmpeg_dir = ffmpeg_dir.ok_or_else(|| {
            format!(
                "FFmpeg not found in {}",
                toolchain::diagnostic_lib_dir().display()
            )
        })?;
        let mut cmd = hidden_command(&yt_dlp_cmd);
        cmd.arg("-f").arg(&option.format_selector);
        if option.extract_audio {
            cmd.args(["-x", "--audio-format", "mp3", "--audio-quality", "192K"]);
        } else {
            cmd.args(["--merge-output-format", "mp4"]);
        }
        cmd.arg("--ignore-config")
            .arg("--encoding")
            .arg("utf-8")
            .arg("--embed-metadata")
            .arg("--newline")
            .arg("--continue")
            .arg("--no-simulate")
            .arg("--progress")
            .arg("--progress-delta")
            .arg("0.5")
            .arg("--progress-template")
            .arg("download:__SHADOW_PROGRESS__%(progress)j")
            .arg("--print")
            .arg("after_move:__SHADOW_FILE__%(filepath)j");
        configure_source(&mut cmd, &platform, auth);
        let sub_langs = extra_subtitle_lang
            .map(|lang| format!("zh,en,{lang}"))
            .unwrap_or_else(|| "zh,en".into());
        cmd.arg("--write-subs").arg("--sub-langs").arg(sub_langs);
        cmd.arg("-o")
            .arg(output_template)
            .arg("--ffmpeg-location")
            .arg(ffmpeg_dir)
            .arg("--no-playlist")
            .arg("--no-warnings")
            .arg(url);
        let mut final_path = None;
        let output = run_command(cmd, control, false, |line| {
            if let Some(value) = line.strip_prefix("__SHADOW_FILE__") {
                final_path = Some(PathBuf::from(
                    serde_json::from_str::<String>(value)
                        .map_err(|error| format!("Invalid download output path: {error}"))?,
                ));
            } else {
                on_progress(line)?;
            }
            Ok(())
        })?;
        if !output.status.success() {
            return Err(format!("yt-dlp: {}", output.stderr.trim()));
        }
        final_path.ok_or_else(|| "下载进程未返回处理完成的文件路径".into())
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    struct FixtureServer {
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    #[test]
    #[ignore = "requires bundled yt-dlp and FFmpeg; downloads a generated WAV from localhost only"]
    fn real_tools_download_and_convert_local_http_audio() {
        let (yt_dlp, ffmpeg) = OnlineResolver::get_tools_paths();
        assert!(Path::new(&yt_dlp).is_file(), "bundled yt-dlp is required");
        assert!(ffmpeg.is_some(), "bundled FFmpeg is required");
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend(16036u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(16000u32.to_le_bytes());
        wav.resize(16044, 0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fixture.wav", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let _server = FixtureServer {
            stop,
            worker: Some(std::thread::spawn(move || {
                while !running.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut bytes = [0u8; 16384];
                            let read = stream.read(&mut bytes).unwrap();
                            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", wav.len());
                            if stream.write_all(header.as_bytes()).is_ok()
                                && !bytes[..read].starts_with(b"HEAD ")
                            {
                                let _ = stream.write_all(&wav);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => panic!("local fixture server: {error}"),
                    }
                }
            })),
        };
        let temp = tempfile::tempdir().unwrap();
        let directory = dunce::canonicalize(temp.path()).unwrap();
        let control = DownloadControl::default();
        let metadata =
            OnlineResolver::resolve_metadata(&url, DownloadAuth::Public, &control).unwrap();
        assert!(!metadata.title.is_empty());
        let options = metadata.download_options().unwrap();
        let option = options
            .iter()
            .find(|option| option.id == "audio:mp3")
            .unwrap();
        let progress = AtomicUsize::new(0);
        let path = OnlineResolver::download_media(
            &DownloadRequest {
                url: &url,
                title: "本地下载测试 100%",
                output_dir: &directory,
                option,
                auth: DownloadAuth::Public,
                extra_subtitle_lang: None,
            },
            &control,
            |line| {
                if crate::services::downloads::parse_progress(line)?.is_some() {
                    progress.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            dunce::canonicalize(&path).unwrap().parent(),
            Some(directory.as_path())
        );
        assert_eq!(path.file_name().unwrap(), "本地下载测试 100%.mp3");
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        assert!(progress.load(Ordering::SeqCst) > 0);
    }
}
