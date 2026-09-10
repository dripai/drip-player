use crate::services::{online_resolver::OnlineResolver, toolchain::hidden_command};
use std::process::Command;
use tauri::{Emitter, Window};
/// 检查外部依赖 (yt-dlp, ffmpeg, ffplay)
#[tauri::command]
pub fn check_dependencies() -> Result<serde_json::Value, String> {
    use crate::services::online_resolver::OnlineResolver;
    use serde_json::json;

    let (yt_dlp_cmd, ffmpeg_dir) = OnlineResolver::get_tools_paths();
    let ffplay_path = OnlineResolver::get_ffplay_path();

    // 检查 yt-dlp
    let yt_dlp_available = hidden_command(&yt_dlp_cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // 检查 ffmpeg
    let ffmpeg_available = if ffmpeg_dir.is_some() {
        OnlineResolver::get_ffmpeg_path().is_some()
    } else {
        false
    };

    // 检查 ffplay
    let ffplay_available = ffplay_path.is_some();

    Ok(json!({
        "yt_dlp": {
            "available": yt_dlp_available,
            "path": yt_dlp_cmd
        },
        "ffmpeg": {
            "available": ffmpeg_available,
            "required": false,
            "purpose": "Video format conversion and merging"
        },
        "ffplay": {
            "available": ffplay_available,
            "required": false,
            "purpose": "External video player"
        }
    }))
}

/// 检查 MPV 是否可用
#[tauri::command]
pub fn check_mpv_available() -> bool {
    OnlineResolver::get_mpv_path().is_some()
}

/// 在默认浏览器中打开平台的登录页面
#[tauri::command]
pub async fn open_platform_login(platform: String) -> Result<String, String> {
    let login_url = match platform.to_lowercase().as_str() {
        "youtube" => "https://accounts.google.com/ServiceLogin?service=youtube",
        "bilibili" | "哔哩哔哩" => "https://passport.bilibili.com/login",
        "douyin" | "抖音" => "https://www.douyin.com/login",
        "tencent" | "腾讯视频" => "https://v.qq.com/",
        "weixin" | "微信视频号" => "https://channels.weixin.qq.com/",
        _ => return Err(format!("Unknown platform: {}", platform)),
    };

    // 在默认浏览器中打开
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", login_url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(login_url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(login_url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    Ok(login_url.to_string())
}

/// 检查错误是否指示需要登录，并返回登录信息
#[tauri::command]
pub fn check_login_required(error: String) -> Option<LoginRequiredInfo> {
    if let Some((platform, login_url, message)) = OnlineResolver::parse_login_error(&error) {
        Some(LoginRequiredInfo {
            platform,
            login_url,
            message,
        })
    } else {
        None
    }
}

#[derive(serde::Serialize)]
pub struct LoginRequiredInfo {
    platform: String,
    login_url: String,
    message: String,
}

/// 尝试使用 OAuth2 认证添加 URL (针对 YouTube)
/// 这将触发基于浏览器的 OAuth 流程
#[tauri::command]
pub async fn add_url_with_oauth(url: String, window: Window) -> Result<(), String> {
    println!("Attempting to add URL with OAuth2: {}", url);

    // 发送解析状态
    window.emit("url-resolving", true).ok();

    let url_clone = url.clone();
    let result = tokio::task::spawn_blocking(move || {
        OnlineResolver::resolve_metadata_with_oauth(&url_clone)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?;

    window.emit("url-resolving", false).ok();

    match result {
        Ok(metadata) => {
            println!("OAuth2 resolved: {} ({})", metadata.title, metadata.id);
            // 现在使用常规流程添加到播放列表
            // 我们需要在这里调用 add_url_for_download 逻辑
            // 目前，发送成功信号并让前端使用正常流程重试
            window.emit("oauth-success", &url).ok();
            Ok(())
        }
        Err(e) => {
            println!("OAuth2 failed: {}", e);
            Err(e)
        }
    }
}
