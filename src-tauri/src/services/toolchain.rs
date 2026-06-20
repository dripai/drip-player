use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

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

pub fn set_resource_dir(path: PathBuf) {
    let _ = RESOURCE_DIR.set(path);
}

pub fn lib_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(resource_dir) = RESOURCE_DIR.get() {
        dirs.push(resource_dir.join("lib"));
    }

    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
    {
        dirs.push(exe_dir.join("lib"));

        #[cfg(target_os = "macos")]
        if let Some(contents_dir) = exe_dir.parent() {
            dirs.push(contents_dir.join("Resources").join("lib"));
        }
    }

    dirs.push(
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("lib"),
    );

    dirs
}

pub fn lib_dir() -> PathBuf {
    lib_dirs()
        .into_iter()
        .find(|dir| dir.exists())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("lib")
        })
}

pub fn executable_name(name: &str) -> String {
    if cfg!(windows) && !name.ends_with(".exe") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

pub fn find_tool(name: &str) -> Option<PathBuf> {
    let executable = executable_name(name);
    lib_dirs()
        .into_iter()
        .map(|dir| dir.join(&executable))
        .find(|path| path.exists())
}

pub fn tool_dir_for(name: &str) -> Option<PathBuf> {
    find_tool(name).and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
}

pub fn diagnostic_lib_dir() -> PathBuf {
    lib_dir()
}

pub fn tool_path(name: &str) -> String {
    find_tool(name)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| lib_dir().join(executable_name(name)).to_string_lossy().to_string())
}

pub fn ffprobe_path() -> String {
    tool_path("ffprobe")
}
