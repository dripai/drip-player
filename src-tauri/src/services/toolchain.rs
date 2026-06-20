use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
pub fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
pub fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

pub fn lib_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("lib")
}

pub fn tool_path(name: &str) -> String {
    let executable = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };
    let bundled = lib_dir().join(&executable);

    if bundled.exists() {
        bundled.to_string_lossy().to_string()
    } else {
        name.to_string()
    }
}

pub fn ffprobe_path() -> String {
    tool_path("ffprobe")
}
