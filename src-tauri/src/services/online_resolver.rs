use std::process::{Command, Stdio};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::io::{BufRead, BufReader};
use crate::models::playlist::MediaType;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
enum AuthStrategy {
    /// No authentication - try without any cookies
    None,
    /// Use cookies from browser (chrome, edge, firefox)
    Browser(&'static str),
    /// Use cookies file
    #[allow(dead_code)]
    CookiesFile(PathBuf),
    /// Use OAuth2 authentication (for YouTube)
    OAuth2,
}

/// Error types for video resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolveError {
    /// Login required - user needs to authenticate
    LoginRequired {
        platform: String,
        login_url: String,
        message: String,
    },
    /// General error
    GeneralError(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::LoginRequired { platform, message, .. } => {
                write!(f, "{} 需要登录: {}", platform, message)
            }
            ResolveError::GeneralError(msg) => write!(f, "{}", msg),
        }
    }
}

/// Supported video platforms with their specific configurations
#[derive(Debug, Clone, PartialEq)]
pub enum VideoPlatform {
    Bilibili,
    YouTube,
    Douyin,      // 抖音
    TencentVideo, // 腾讯视频
    Weixin,      // 微信视频号
    Generic,     // 通用/其他平台
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
        } else if url_lower.contains("channels.weixin.qq.com") || url_lower.contains("finder.video.qq.com") {
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

    /// Check if this platform typically requires cookies for full access
    pub fn needs_cookies(&self) -> bool {
        match self {
            VideoPlatform::Bilibili => true,  // For high quality
            VideoPlatform::YouTube => true,   // For age-restricted content
            VideoPlatform::Douyin => true,    // Often needs login
            VideoPlatform::TencentVideo => true,
            VideoPlatform::Weixin => true,
            VideoPlatform::Generic => false,
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
        } else if url_lower.contains("douyinvod") || url_lower.contains("bytedance") || url_lower.contains("amemv") {
            Some(VideoPlatform::Douyin)
        } else if url_lower.contains("v.qq.com") || url_lower.contains("gtimg.com") {
            Some(VideoPlatform::TencentVideo)
        } else {
            None
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct VideoMetadata {
    pub title: String,
    pub duration: Option<f64>, // seconds
    pub id: String,
    pub vcodec: Option<String>,
    #[allow(dead_code)]
    pub webpage_url: String,
}

impl VideoMetadata {
    #[allow(dead_code)]
    pub fn get_media_type(&self) -> MediaType {
        match &self.vcodec {
            Some(v) if v != "none" => MediaType::Video,
            _ => MediaType::Audio,
        }
    }
}

pub struct OnlineResolver;

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
            let end: String = s.chars().rev().take(end_take).collect::<String>().chars().rev().collect();
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
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lib_dir = current_dir.join("lib");
        
        let yt_dlp_path = lib_dir.join("yt-dlp.exe");
        let ffmpeg_path = lib_dir.join("ffmpeg.exe");
        
        let yt_dlp_cmd = if yt_dlp_path.exists() {
            yt_dlp_path.to_string_lossy().to_string()
        } else {
            "yt-dlp".to_string()
        };
        
        let ffmpeg_cmd = if ffmpeg_path.exists() {
            Some(lib_dir.to_string_lossy().to_string()) // yt-dlp expects directory or executable for --ffmpeg-location
        } else {
            None
        };
        
        (yt_dlp_cmd, ffmpeg_cmd)
    }

    pub fn get_ffplay_path() -> Option<String> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lib_dir = current_dir.join("lib");
        let ffplay_path = lib_dir.join("ffplay.exe");

        if ffplay_path.exists() {
            Some(ffplay_path.to_string_lossy().to_string())
        } else {
            // Check system PATH
            let in_path = hidden_command("ffplay")
                .arg("-version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if in_path {
                Some("ffplay".to_string())
            } else {
                None
            }
        }
    }

    pub fn get_mpv_path() -> Option<String> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lib_dir = current_dir.join("lib");
        let mpv_path = lib_dir.join("mpv.exe");

        if mpv_path.exists() {
            Some(mpv_path.to_string_lossy().to_string())
        } else {
            // Check system PATH
            let in_path = hidden_command("mpv")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if in_path {
                Some("mpv".to_string())
            } else {
                None
            }
        }
    }

    pub fn get_ffmpeg_path() -> Option<String> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lib_dir = current_dir.join("lib");
        let ffmpeg_path = lib_dir.join("ffmpeg.exe");
        
        if ffmpeg_path.exists() {
            Some(ffmpeg_path.to_string_lossy().to_string())
        } else {
            // Check system PATH
            let in_path = hidden_command("ffmpeg")
                .arg("-version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if in_path {
                Some("ffmpeg".to_string())
            } else {
                None
            }
        }
    }

    fn is_ffmpeg_available() -> bool {
        let (yt_dlp_cmd, ffmpeg_path) = Self::get_tools_paths();
        if let Some(path) = ffmpeg_path {
            println!("Found ffmpeg in lib: {}", path);
            return true;
        }
        
        // Derive lib path from yt-dlp path or default to current dir/lib
        let lib_path = if yt_dlp_cmd == "yt-dlp" {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("lib")
        } else {
             Path::new(&yt_dlp_cmd).parent().unwrap_or(Path::new(".")).to_path_buf()
        };

        // Print message about missing ffmpeg in lib
        println!("当前路径({}) 未检测到ffmpeg.exe文件", lib_path.display());

        // Check system PATH
        let in_path = hidden_command("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
            
        if in_path {
            println!("Found ffmpeg in system PATH");
        } else {
            println!("ffmpeg not found in system PATH either");
        }
        
        in_path
    }

    pub fn get_stream_url(url: &str) -> Result<String, String> {
        let (yt_dlp_cmd, _) = Self::get_tools_paths();
        let platform = VideoPlatform::from_url(url);

        println!("Resolving stream URL for platform: {} ({})", platform.display_name(), url);

        // Use the same strategies as resolve_metadata to bypass 412 errors
        let strategies = if platform.needs_cookies() {
            vec![
                AuthStrategy::Browser("chrome"),
                AuthStrategy::Browser("edge"),
                AuthStrategy::Browser("firefox"),
                AuthStrategy::None
            ]
        } else {
            vec![AuthStrategy::None]
        };
        let mut last_error = String::new();

        for strategy in strategies {
            let mut cmd = hidden_command(&yt_dlp_cmd);
            cmd.args(&[
                "-g",
                "-f", "best[ext=mp4]/best", // Prefer mp4, fallback to best
                "--user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            ]);

            // For YouTube, add special options to help bypass bot detection
            if platform == VideoPlatform::YouTube {
                cmd.arg("--extractor-args")
                   .arg("youtube:player_client=web,default");
            }

            // Add platform-specific referer if needed
            if let Some(referer) = platform.get_referer() {
                cmd.args(&["--referer", referer]);
            }

            match &strategy {
                AuthStrategy::Browser(b) => {
                    cmd.arg("--cookies-from-browser").arg(b);
                },
                AuthStrategy::CookiesFile(p) => {
                    cmd.arg("--cookies").arg(p);
                },
                AuthStrategy::None => {},
                AuthStrategy::OAuth2 => {
                    // OAuth2 handled separately
                    continue;
                }
            }

            let output = cmd.arg(url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

            if output.status.success() {
                 let video_url = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                println!("Successfully resolved stream URL for {}", platform.display_name());
                return Ok(video_url);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = stderr.to_string();
                // If it's a cookie lock error, try next browser
                if stderr.contains("Could not copy") || stderr.contains("Sign in") {
                    continue;
                }
            }
        }

        Err(format!("yt-dlp failed for {} after retries: {}", platform.display_name(), last_error))
    }

    pub fn resolve_metadata(url: &str) -> Result<VideoMetadata, String> {
        let (yt_dlp_cmd, _) = Self::get_tools_paths();
        let platform = VideoPlatform::from_url(url);

        println!("Resolving metadata for platform: {} ({})", platform.display_name(), url);

        // Strategy order:
        // 1. No auth (try without cookies first)
        // 2. Browser cookies (chrome, edge, firefox)
        // 3. If all fail and login required, return special error for OAuth/manual login
        let strategies = vec![
            AuthStrategy::None,  // Try without auth first
            AuthStrategy::Browser("chrome"),
            AuthStrategy::Browser("edge"),
            AuthStrategy::Browser("firefox"),
        ];
        let mut last_error = String::new();
        let mut needs_login = false;

        for strategy in &strategies {
            let strategy_name = match strategy {
                AuthStrategy::None => "none".to_string(),
                AuthStrategy::Browser(b) => format!("browser:{}", b),
                AuthStrategy::CookiesFile(p) => format!("cookies:{}", p.display()),
                AuthStrategy::OAuth2 => "oauth2".to_string(),
            };
            println!("Trying strategy: {}", strategy_name);

            let mut cmd = hidden_command(&yt_dlp_cmd);
            cmd.arg("--dump-json")
               .arg("--no-playlist")
               .arg("--no-warnings")
               .arg("--user-agent")
               .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");

            // For YouTube, add special options to help bypass bot detection
            if platform == VideoPlatform::YouTube {
                cmd.arg("--extractor-args")
                   .arg("youtube:player_client=web,default");
            }

            // Add platform-specific referer if needed
            if let Some(referer) = platform.get_referer() {
                cmd.arg("--referer").arg(referer);
            }

            match strategy {
                AuthStrategy::None => {},
                AuthStrategy::Browser(b) => {
                    cmd.arg("--cookies-from-browser").arg(*b);
                },
                AuthStrategy::CookiesFile(p) => {
                    cmd.arg("--cookies").arg(p);
                },
                AuthStrategy::OAuth2 => {
                    // OAuth2 is handled separately as it requires user interaction
                }
            }

            let output = cmd.arg(url)
                .output()
                .map_err(|e| format!("Failed to execute yt-dlp: {}", e))?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);

                // Try to parse each line as JSON
                for line in stdout.lines() {
                    if let Ok(metadata) = serde_json::from_str::<VideoMetadata>(line) {
                        println!("Successfully resolved metadata for {}: {} (strategy: {})", platform.display_name(), metadata.title, strategy_name);
                        return Ok(metadata);
                    }
                }

                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = format!("Failed to parse JSON from output. Stderr: {}", stderr);
                println!("Strategy {} failed: {}", strategy_name, last_error);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = stderr.to_string();
                println!("Strategy {} failed: {}", strategy_name, last_error);

                // Check if this is a login-related error
                if stderr.contains("Sign in") || stderr.contains("bot") || stderr.contains("login") {
                    needs_login = true;
                }

                // Continue to next strategy
                if stderr.contains("Could not copy") {
                    // Browser cookie lock error, try next browser
                    continue;
                }
            }
        }

        // If login is needed, return special error code
        if needs_login && platform.needs_cookies() {
            // Return error that indicates OAuth should be tried
            Err(format!("LOGIN_REQUIRED:{}:{}:{}",
                platform.display_name(),
                platform.get_login_url(),
                last_error))
        } else {
            Err(format!("yt-dlp error for {} after retries: {}", platform.display_name(), last_error))
        }
    }

    /// Try to resolve metadata using OAuth2 authentication
    /// This will open a browser for user to authorize
    /// Returns Ok(metadata) if successful, Err with message if failed
    pub fn resolve_metadata_with_oauth(url: &str) -> Result<VideoMetadata, String> {
        let (yt_dlp_cmd, _) = Self::get_tools_paths();
        let platform = VideoPlatform::from_url(url);

        // OAuth2 is primarily for YouTube
        if platform != VideoPlatform::YouTube {
            return Err(format!("OAuth2 is only supported for YouTube, not {}", platform.display_name()));
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
                    println!("Successfully resolved metadata with OAuth2: {}", metadata.title);
                    return Ok(metadata);
                }
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("OAuth2 succeeded but failed to parse response. Stderr: {}", stderr))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("OAuth2 failed: {}", stderr);
            Err(format!("OAuth2 authentication failed: {}", stderr))
        }
    }

    /// Check if an error indicates login is required
    pub fn is_login_required_error(error: &str) -> bool {
        error.starts_with("LOGIN_REQUIRED:")
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

    pub fn download_media<F>(url: &str, id: &str, title: &str, output_dir: &Path, media_type: MediaType, extra_subtitle_lang: Option<&str>, on_progress: F) -> Result<PathBuf, String>
    where F: Fn(String) + Send + 'static + Clone
    {
        let platform = VideoPlatform::from_url(url);

        println!("Starting download for platform: {} ({})", platform.display_name(), url);

        let strategies = if platform.needs_cookies() {
            vec![
                AuthStrategy::Browser("chrome"),
                AuthStrategy::Browser("edge"),
                AuthStrategy::Browser("firefox"),
                AuthStrategy::None
            ]
        } else {
            vec![AuthStrategy::None]
        };
        let mut last_error = String::new();

        for strategy in strategies {
            let res = Self::download_media_internal(url, id, title, output_dir, media_type.clone(), extra_subtitle_lang, on_progress.clone(), &strategy, &platform);
            match res {
                Ok(path) => return Ok(path),
                Err(e) => {
                    last_error = e.clone();
                    if e.contains("Could not copy") || e.contains("Sign in") {
                        continue;
                    }
                    // For download, we might want to be more persistent, so just try next
                }
            }
        }

        Err(format!("Download failed for {} after retries: {}", platform.display_name(), last_error))
    }

    fn download_media_internal<F>(url: &str, id: &str, title: &str, output_dir: &Path, media_type: MediaType, extra_subtitle_lang: Option<&str>, on_progress: F, strategy: &AuthStrategy, platform: &VideoPlatform) -> Result<PathBuf, String>
    where F: Fn(String) + Send + 'static
    {
        // Check for existing file with same ID (ignoring extension)
        if !output_dir.exists() {
            std::fs::create_dir_all(output_dir).map_err(|e| format!("Failed to create cache dir: {}", e))?;
        } else if let Ok(entries) = std::fs::read_dir(output_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().unwrap().to_string_lossy();
                    // Check if filename matches the sanitized title (possibly with a numeric suffix) or contains the id
                    let safe_title = Self::sanitize_filename(title);
                    let stem = name.split('.').next().unwrap_or("");
                    let title_match = stem == safe_title || stem.starts_with(&format!("{} - ", safe_title)) || stem.contains(&safe_title);
                    if (title_match || name.contains(id) || name == id) && !name.ends_with(".tmp") && !name.ends_with(".part") && !name.ends_with(".ytdl") {
                        // If we want video, ignore audio-only files
                        if media_type == MediaType::Video {
                            if let Some(ext) = path.extension() {
                                let ext_str = ext.to_string_lossy().to_lowercase();
                                if ["mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"].contains(&ext_str.as_str()) {
                                    continue;
                                }
                            }
                        }

                        // Found existing file
                        on_progress(format!("File already exists: {}", name));
                        return Ok(path);
                    }
                }
            }
        }

        // Use template including a sanitized title only (user requested). yt-dlp will write <title>.<ext>
        let safe_title = Self::sanitize_filename(title);
        let output_template = output_dir.join(format!("{}.%(ext)s", safe_title));

        let (yt_dlp_cmd, ffmpeg_dir) = Self::get_tools_paths();

        let mut cmd = hidden_command(&yt_dlp_cmd);
        
        // Log tool detection status
        if Self::is_ffmpeg_available() {
            println!("Starting download with ffmpeg support");
        } else {
            println!("Starting download WITHOUT ffmpeg support (fallback mode)");
        }
        
        match media_type {
            MediaType::Video => {
                 if Self::is_ffmpeg_available() {
                     cmd.arg("-f")
                        .arg("bestvideo+bestaudio/best");
                 } else {
                     // Fallback to best single file if no ffmpeg (avoid merge)
                     cmd.arg("-f")
                        .arg("best");
                 }
            },
            MediaType::Audio => {
                cmd.arg("-x")
                    .arg("--audio-format")
                    .arg("mp3")
                    .arg("--audio-quality")
                    .arg("192K");
            }
        }

        cmd.arg("--embed-metadata")
            .arg("--newline")
            .arg("--user-agent")
            .arg("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");

        // Add platform-specific referer if needed
        if let Some(referer) = platform.get_referer() {
            cmd.arg("--referer").arg(referer);
        }

        // Add subtitle download options: default zh,en, plus optional extra language
        // yt-dlp will skip if subtitles are not available (no error)
        cmd.arg("--write-subs")
            .arg("--sub-langs");

        let sub_langs = if let Some(extra_lang) = extra_subtitle_lang {
            format!("zh,en,{}", extra_lang)
        } else {
            "zh,en".to_string()
        };
        cmd.arg(&sub_langs);

        match strategy {
            AuthStrategy::Browser(b) => {
                cmd.arg("--cookies-from-browser").arg(b);
            },
            AuthStrategy::CookiesFile(p) => {
                cmd.arg("--cookies").arg(p);
            },
            AuthStrategy::None => {},
            AuthStrategy::OAuth2 => {
                // OAuth2 not used in download
            }
        }
            
        cmd.arg("-o")
            .arg(output_template.to_string_lossy().as_ref());
            
        if let Some(ffmpeg) = ffmpeg_dir {
            cmd.arg("--ffmpeg-location").arg(ffmpeg);
        }
            
        let mut child = cmd.arg("--no-playlist")
            .arg("--no-warnings")
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn yt-dlp: {}", e))?;

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("[yt-dlp] {}", line);
                    if line.contains("[download]") {
                        on_progress(line);
                    }
                }
            }
        }
        
        let status = child.wait().map_err(|e| format!("Failed to wait on yt-dlp: {}", e))?;

        if !status.success() {
            let mut stderr_msg = String::new();
            if let Some(stderr) = child.stderr.take() {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        stderr_msg.push_str(&line);
                        stderr_msg.push('\n');
                        println!("[yt-dlp error] {}", line);
                    }
                }
            }
            return Err(format!("yt-dlp download error: {}", stderr_msg));
        }

        // Find the downloaded file. It should match "{id}.*" but not be a temp file or subtitle
        let mut downloaded_path = None;
        if let Ok(entries) = std::fs::read_dir(output_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name() {
                            let name_str = name.to_string_lossy();
                                    // Match downloaded media files by sanitized title (and possible numeric suffix) or by id
                                    let stem = name_str.split('.').next().unwrap_or("").to_string();
                                    let title_match = stem == safe_title || stem.starts_with(&format!("{} - ", safe_title)) || stem.contains(&safe_title);
                                    if (title_match || name_str.contains(id) || name_str == id) &&
                               !name_str.ends_with(".part") &&
                               !name_str.ends_with(".ytdl") &&
                               !name_str.ends_with(".tmp") &&
                               !name_str.ends_with(".srt") &&
                               !name_str.ends_with(".vtt") &&
                               !name_str.ends_with(".ass") &&
                               !name_str.ends_with(".ssa") {
                                downloaded_path = Some(path);
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(path) = downloaded_path {
            println!("Download completed: {}", path.display());
            Ok(path)
        } else {
            Err(format!("Output file not found after download for title: {}", safe_title))
        }
    }
}
