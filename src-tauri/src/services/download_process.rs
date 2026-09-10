use crate::app_state::lock;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Default)]
pub struct DownloadControl {
    canceled: AtomicBool,
    // The worker releases this before any database/directory operation.
    process: Mutex<Option<String>>,
}

impl DownloadControl {
    pub fn check(&self) -> Result<(), String> {
        if self.canceled.load(Ordering::SeqCst) {
            Err("下载任务已停止".into())
        } else {
            Ok(())
        }
    }
    pub fn cancel_and_wait(&self) -> Result<(), String> {
        self.canceled.store(true, Ordering::SeqCst);
        if let Some(error) = &*lock(&self.process)? {
            return Err(error.clone());
        }
        Ok(())
    }
}

pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

// Called only from blocking workers. The async pipes drain concurrently, including during FFmpeg.
pub fn run_command(
    mut command: Command,
    control: &DownloadControl,
    capture_stdout: bool,
    mut on_line: impl FnMut(&str) -> Result<(), String>,
) -> Result<CommandOutput, String> {
    let mut process = lock(&control.process)?;
    control.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    tauri::async_runtime::block_on(async {
        let mut wrapped = CommandWrap::from(tokio::process::Command::from(command));
        wrapped.wrap(KillOnDrop);
        #[cfg(windows)]
        wrapped
            .wrap(process_wrap::tokio::CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ))
            .wrap(process_wrap::tokio::JobObject);
        #[cfg(unix)]
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        let mut child = wrapped
            .spawn()
            .map_err(|error| format!("无法启动下载进程：{error}"))?;
        let mut stdout = BufReader::new(
            child
                .stdout()
                .take()
                .ok_or("Download stdout is unavailable")?,
        )
        .lines();
        let mut stderr = BufReader::new(
            child
                .stderr()
                .take()
                .ok_or("Download stderr is unavailable")?,
        )
        .lines();
        let (mut stdout_end, mut stderr_end) = (false, false);
        let (mut output, mut errors) = (String::new(), String::new());
        let result: Result<ExitStatus, String> = async {
            loop {
                control.check()?;
                let next = tokio::select! {
                    line = stdout.next_line(), if !stdout_end => Some((false, line)),
                    line = stderr.next_line(), if !stderr_end => Some((true, line)),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => None,
                };
                if let Some((is_error, line)) = next {
                    match line.map_err(|error| format!("读取下载输出失败：{error}"))? {
                        Some(line) => {
                            on_line(&line)?;
                            if is_error || capture_stdout {
                                let target = if is_error { &mut errors } else { &mut output };
                                if is_error && target.len() > 16 * 1024 {
                                    target.clear();
                                }
                                if !is_error && target.len() > 16 * 1024 * 1024 {
                                    return Err("媒体元数据过大".into());
                                }
                                target.push_str(&line);
                                target.push('\n');
                            }
                        }
                        None => {
                            if is_error {
                                stderr_end = true;
                            } else {
                                stdout_end = true;
                            }
                        }
                    }
                }
                if stdout_end && stderr_end {
                    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                        return Ok(status);
                    }
                }
            }
        }
        .await;
        if result.is_err() {
            let stopped = match child.start_kill() {
                Ok(()) => child.wait().await.map(|_| ()),
                Err(error) => Err(error),
            };
            if let Err(error) = stopped {
                let error = format!("停止下载进程失败：{error}");
                *process = Some(error.clone());
                return Err(error);
            }
        }
        Ok(CommandOutput {
            status: result?,
            stdout: output,
            stderr: errors,
        })
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    pub fn fixture_command(path: &Path, role: &str) -> Command {
        let executable = std::env::current_exe().unwrap();
        let mut command = crate::services::toolchain::hidden_command(executable.to_str().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "services::download_process::tests::process_fixture",
                "--nocapture",
            ])
            .env("SHADOW_DOWNLOAD_FIXTURE_PATH", path)
            .env("SHADOW_DOWNLOAD_FIXTURE_ROLE", role);
        command
    }

    #[test]
    #[ignore = "subprocess fixture, invoked by process cancellation tests"]
    fn process_fixture() {
        let path =
            std::path::PathBuf::from(std::env::var_os("SHADOW_DOWNLOAD_FIXTURE_PATH").unwrap());
        match std::env::var("SHADOW_DOWNLOAD_FIXTURE_ROLE")
            .unwrap()
            .as_str()
        {
            "child" => {
                let mut file = std::fs::File::create(&path).unwrap();
                for _ in 0..400 {
                    file.write_all(b"x").unwrap();
                    file.flush().unwrap();
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            "owner" => {
                let control = DownloadControl::default();
                run_command(
                    fixture_command(&path, "parent"),
                    &control,
                    false,
                    |_| Ok(()),
                )
                .unwrap();
            }
            _ => {
                let mut child = fixture_command(&path, "child").spawn().unwrap();
                println!("SHADOW_FIXTURE_READY");
                std::io::stdout().flush().unwrap();
                child.wait().unwrap();
            }
        }
    }

    pub fn start_fixture(
        control: Arc<DownloadControl>,
        path: &Path,
    ) -> std::thread::JoinHandle<Result<CommandOutput, String>> {
        let command = fixture_command(path, "parent");
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_command(command, &control, false, |line| {
                if line == "SHADOW_FIXTURE_READY" {
                    sender.send(()).unwrap();
                }
                Ok(())
            })
        });
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        wait_for_file(path);
        worker
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.is_file() {
            assert!(
                Instant::now() < deadline,
                "child fixture never wrote a file"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn assert_file_stopped(path: &Path) {
        let before = std::fs::metadata(path).unwrap().len();
        std::thread::sleep(Duration::from_millis(180));
        assert_eq!(
            before,
            std::fs::metadata(path).unwrap().len(),
            "descendant continued writing"
        );
    }

    #[test]
    fn cancellation_stops_descendants_and_prevents_a_later_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("child-writing");
        let control = Arc::new(DownloadControl::default());
        let worker = start_fixture(control.clone(), &marker);
        control.cancel_and_wait().unwrap();
        assert!(worker.join().unwrap().is_err());
        assert_file_stopped(&marker);
        let another = temp.path().join("must-not-start");
        assert!(
            run_command(fixture_command(&another, "child"), &control, false, |_| Ok(
                ()
            ))
            .is_err()
        );
        assert!(!another.exists());
    }

    #[cfg(windows)]
    #[test]
    fn forced_application_exit_closes_the_job_and_stops_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("child-writing");
        let mut owner = fixture_command(&marker, "owner")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for_file(&marker);
        owner.kill().unwrap();
        owner.wait().unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert_file_stopped(&marker);
    }
}
