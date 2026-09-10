use crate::app_state::{lock, AppState};
use crate::models::download::{
    DownloadAction, DownloadAuth, DownloadFile, DownloadJob, DownloadOption, DownloadPhase,
    DownloadProgress, DownloadSnapshot,
};
use crate::models::media::{AssetKind, AssetSource, Media, MediaAsset, MediaOrigin};
use crate::services::directory_library::{contains_path, DirectorySnapshot};
use crate::services::download_process::DownloadControl;
use crate::services::file_operations::{move_file, stamp};
use crate::services::online_resolver::{DownloadRequest, OnlineResolver};
use crate::services::{
    download_options::duration_tolerance, media_assets, media_probe,
    persistence::PersistenceManager, toolchain,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

struct RuntimeTask {
    attempt: u64,
    control: Arc<DownloadControl>,
    progress: Option<DownloadProgress>,
    error: Option<String>,
}

#[derive(Default)]
pub struct DownloadRegistry {
    tasks: HashMap<String, RuntimeTask>,
    revision: u64,
}

pub fn snapshot(state: &AppState) -> Result<DownloadSnapshot, String> {
    let database = lock(&state.database)?;
    let mut registry = lock(&state.downloads)?;
    let mut jobs = database.download_jobs()?;
    for job in &mut jobs {
        if let Some(runtime) = registry
            .tasks
            .get(&job.id)
            .filter(|task| task.attempt == job.attempt)
        {
            if job.phase.is_active() {
                job.progress = runtime.progress.clone();
            }
            if let Some(error) = &runtime.error {
                job.error = Some(error.clone());
                job.phase = DownloadPhase::RecoveryRequired;
            }
        }
    }
    registry.revision = registry
        .revision
        .checked_add(1)
        .ok_or("Download revision overflow")?;
    Ok(DownloadSnapshot {
        revision: registry.revision,
        jobs,
    })
}

pub fn notify(app: &AppHandle) {
    if let Err(error) = app.emit("downloads-updated", ()) {
        eprintln!("Download notification failed: {error}");
    }
}

pub fn clear_history(state: &AppState) -> Result<usize, String> {
    // Serialize with starting, canceling and publishing. Selection uses committed
    // phases plus the same terminal runtime-error override shown by snapshot().
    let _operation = lock(&state.directory_operation)?;
    let mut database = lock(&state.database)?;
    let mut registry = lock(&state.downloads)?;
    let ids: Vec<_> = database
        .download_jobs()?
        .into_iter()
        .filter(|job| {
            !job.phase.is_active()
                || registry
                    .tasks
                    .get(&job.id)
                    .is_some_and(|task| task.attempt == job.attempt && task.error.is_some())
        })
        .map(|job| job.id)
        .collect();
    if ids.is_empty() {
        return Ok(0);
    }
    database.remove_download_history(&ids)?;
    for id in &ids {
        registry.tasks.remove(id);
    }
    Ok(ids.len())
}

// Recover the rename journal before startup scanning or a manual retry.
fn restore_publication(job: &DownloadJob) -> Result<(), String> {
    let files = publication_paths(job)?;
    let root = job
        .directory
        .as_deref()
        .ok_or("Missing download directory")?;
    // Validate every surviving file before rolling anything back. A target is
    // owned by this journal only when its original source is gone and stamp matches.
    for ((source, output), entry) in files.iter().zip(&job.publication) {
        let source_exists = source.try_exists().map_err(|e| e.to_string())?;
        if source_exists
            && output.try_exists().map_err(|e| e.to_string())?
            && stamp(output)? == entry.stamp
        {
            // Some non-Windows no-clobber moves use hard links. If unlinking
            // fails, two names can survive; never silently call that a rollback.
            return Err(format!(
                "下载恢复失败，源文件与同版本目标文件同时存在：{}、{}",
                source.display(),
                output.display()
            ));
        }
        let path = if source_exists { source } else { output };
        verify_child(root, path)?;
        if stamp(path)? != entry.stamp {
            return Err(format!("下载恢复失败，文件已被修改：{}", path.display()));
        }
    }
    for (source, output) in files.iter().rev() {
        if !source.try_exists().map_err(|e| e.to_string())? {
            move_file(output, source)?;
        }
    }
    Ok(())
}

pub fn recover(database: &PersistenceManager) -> Result<(), String> {
    for mut job in database.download_jobs()? {
        if matches!(
            job.phase,
            DownloadPhase::Publishing | DownloadPhase::RecoveryRequired
        ) {
            restore_publication(&job)?;
        }
        if job.phase.is_active() || job.phase == DownloadPhase::RecoveryRequired {
            job.phase = DownloadPhase::Interrupted;
            job.interrupt_reason = Some("app_closed".into());
            job.output_path = None;
            job.publication.clear();
            job.error = None;
            database.save_download_job(&job)?;
        }
    }
    Ok(())
}

pub fn recover_publications(database: &PersistenceManager) -> Result<(), String> {
    for mut job in database.download_jobs()? {
        if matches!(
            job.phase,
            DownloadPhase::Publishing | DownloadPhase::RecoveryRequired
        ) {
            restore_publication(&job)?;
            job.phase = DownloadPhase::Interrupted;
            job.output_path = None;
            job.publication.clear();
            database.save_download_job(&job)?;
        }
    }
    Ok(())
}

fn prepare_attempt(
    state: &AppState,
    id: &str,
    action: &DownloadAction,
) -> Result<(DownloadJob, DirectorySnapshot, Arc<DownloadControl>), String> {
    let database = lock(&state.database)?;
    let mut registry = lock(&state.downloads)?;
    let mut job = database.download_job(id)?;
    if matches!(
        job.phase,
        DownloadPhase::Publishing | DownloadPhase::RecoveryRequired
    ) {
        // An active publisher holds directory_operation, so it cannot reach here concurrently.
        restore_publication(&job)?;
        job.phase = DownloadPhase::Interrupted;
        database.save_download_job(&job)?;
    }
    if registry
        .tasks
        .get(id)
        .is_some_and(|task| task.error.is_some())
        && job.phase.is_active()
    {
        database.finish_download_attempt(
            id,
            job.attempt,
            DownloadPhase::Interrupted,
            None,
            None,
        )?;
    }
    let directory = database.directory()?;
    if !directory.path.is_dir() {
        return Err(format!("保存目录不可用：{}", directory.path.display()));
    }
    if dunce::canonicalize(&directory.path).map_err(|error| error.to_string())? != directory.path {
        return Err("保存目录路径已发生变化，请在设置中重新选择".into());
    }
    let job = database.start_download_attempt(id, action)?;
    let control = Arc::new(DownloadControl::default());
    registry.tasks.insert(
        job.id.clone(),
        RuntimeTask {
            attempt: job.attempt,
            control: control.clone(),
            progress: None,
            error: None,
        },
    );
    Ok((job, directory, control))
}

pub fn submit(
    state: &AppState,
    app: &AppHandle,
    url: String,
    auth: DownloadAuth,
) -> Result<String, String> {
    let url = url.trim().to_string();
    crate::models::media::provider_key_for_url(&url)?;
    let _operation = lock(&state.directory_operation)?;
    {
        let database = lock(&state.database)?;
        if let Some(job) = database.download_jobs()?.iter().find(|job| {
            job.url == url
                && job.auth == auth
                && (job.phase.is_active() || job.phase == DownloadPhase::AwaitingSelection)
        }) {
            return Ok(job.id.clone());
        }
    }
    let job = DownloadJob::new(url)?;
    lock(&state.database)?.save_download_job(&job)?;
    let prepared = prepare_attempt(state, &job.id, &DownloadAction::Resolve(auth))?;
    launch(state.clone(), app.clone(), prepared);
    Ok(job.id)
}

pub fn start(
    state: &AppState,
    app: &AppHandle,
    id: &str,
    action: DownloadAction,
) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let prepared = prepare_attempt(state, id, &action)?;
    launch(state.clone(), app.clone(), prepared);
    Ok(())
}

fn launch(
    state: AppState,
    app: AppHandle,
    (job, directory, control): (DownloadJob, DirectorySnapshot, Arc<DownloadControl>),
) {
    notify(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_download(&state, &app, &job, &directory, &control)
        }))
        .unwrap_or_else(|_| Err("下载任务异常退出".into()));
        let finish = finish_attempt(&state, &job, result);
        if let Err(error) = finish {
            eprintln!("Failed to finalize download {}: {error}", job.id);
            if let Ok(mut registry) = lock(&state.downloads) {
                if let Some(task) = registry
                    .tasks
                    .get_mut(&job.id)
                    .filter(|task| task.attempt == job.attempt)
                {
                    task.error = Some(error);
                }
            }
        }
        notify(&app);
        if let Err(error) = app.emit("playlist-updated", ()) {
            eprintln!("Playlist notification failed: {error}");
        }
    });
}

fn finish_attempt(
    state: &AppState,
    job: &DownloadJob,
    result: Result<(), String>,
) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    // Cleared records and superseded attempts must not be recreated by late callbacks.
    if lock(&state.downloads)?
        .tasks
        .get(&job.id)
        .is_none_or(|task| task.attempt != job.attempt)
    {
        return Ok(());
    }
    let database = lock(&state.database)?;
    if let Err(error) = result {
        let current = database.download_job(&job.id)?;
        if current.attempt == job.attempt && current.phase.is_active() {
            let (phase, error) = if current.phase == DownloadPhase::Publishing {
                match restore_publication(&current) {
                    Ok(()) => (DownloadPhase::Failed, error),
                    Err(recovery) => (
                        DownloadPhase::RecoveryRequired,
                        format!("{error}; {recovery}"),
                    ),
                }
            } else {
                (DownloadPhase::Failed, error)
            };
            database.finish_download_attempt(&job.id, job.attempt, phase, Some(error), None)?;
        }
    }
    lock(&state.downloads)?.tasks.remove(&job.id);
    Ok(())
}

// Caller holds directory_operation. Never wait while holding playback, database or registry locks.
pub fn interrupt_all(state: &AppState, reason: &str) -> Result<(), String> {
    let controls: Vec<_> = lock(&state.downloads)?
        .tasks
        .values()
        .map(|task| task.control.clone())
        .collect();
    for control in controls {
        control.cancel_and_wait()?;
    }
    let database = lock(&state.database)?;
    recover_publications(&database)?;
    for job in database.download_jobs()? {
        if job.phase.is_active() {
            database.finish_download_attempt(
                &job.id,
                job.attempt,
                DownloadPhase::Interrupted,
                None,
                Some(reason.into()),
            )?;
        }
    }
    Ok(())
}

pub fn cancel(state: &AppState, id: &str) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let control = lock(&state.downloads)?
        .tasks
        .get(id)
        .map(|task| task.control.clone());
    if let Some(control) = control {
        control.cancel_and_wait()?;
    }
    let database = lock(&state.database)?;
    let mut job = database.download_job(id)?;
    if job.phase == DownloadPhase::AwaitingSelection {
        job.phase = DownloadPhase::Canceled;
        database.save_download_job(&job)?;
    } else {
        database.finish_download_attempt(id, job.attempt, DownloadPhase::Canceled, None, None)?;
    }
    Ok(())
}

fn run_download(
    state: &AppState,
    app: &AppHandle,
    job: &DownloadJob,
    directory: &DirectorySnapshot,
    control: &DownloadControl,
) -> Result<(), String> {
    control.check()?;
    let metadata = OnlineResolver::resolve_metadata(&job.url, job.auth, control)?;
    let options = metadata.download_options()?;
    let kind = options
        .first()
        .ok_or("没有可用的下载格式")?
        .media_type
        .clone();
    let title = metadata.display_title();
    let expected_duration = metadata.expected_duration();
    let draft = crate::application::remote_media(job.url.clone(), title, metadata.id, kind)?;
    let (media, current) = {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        let mut database = lock(&state.database)?;
        database.verify_directory(directory)?;
        let media = database.resolve_download_media(
            &job.id,
            job.attempt,
            &draft,
            options,
            expected_duration,
        )?;
        (media, database.download_job(&job.id)?)
    };
    notify(app);
    if current.phase == DownloadPhase::AwaitingSelection {
        return Ok(());
    }
    let option = current
        .options
        .iter()
        .find(|option| Some(&option.id) == current.selection.as_ref())
        .ok_or("Download format is not selected")?;
    let staging = staging_path(&current)?;
    {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        lock(&state.database)?.verify_directory(directory)?;
        prepare_staging(&current, option, &staging)?;
    }
    if !matches!(media.origin, MediaOrigin::Remote { .. }) {
        return Err("Expected remote media".into());
    }
    let path = OnlineResolver::download_media(
        &DownloadRequest {
            url: &job.url,
            title: &media.title,
            output_dir: &staging,
            option,
            auth: current.auth,
            extra_subtitle_lang: None,
        },
        control,
        |line| {
            if let Some(progress) = parse_progress(line)? {
                let mut registry = lock(&state.downloads)?;
                if let Some(task) = registry
                    .tasks
                    .get_mut(&job.id)
                    .filter(|task| task.attempt == job.attempt)
                {
                    task.progress = Some(progress);
                }
                drop(registry);
                notify(app);
            }
            Ok(())
        },
    )?;
    {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        let database = lock(&state.database)?;
        database.verify_directory(directory)?;
        let mut verifying = database.download_job(&job.id)?;
        if verifying.attempt != job.attempt || verifying.phase != DownloadPhase::Downloading {
            return Err("下载任务已停止".into());
        }
        if dunce::simplified(&path).parent() != Some(staging.as_path()) || !path.is_file() {
            return Err("下载工具返回了任务目录之外的文件".into());
        }
        verify_child(&directory.path, &path)?;
        verifying.phase = DownloadPhase::Verifying;
        database.save_download_job(&verifying)?;
    }
    notify(app);
    if let Err(error) = verify_download(&path, option, expected_duration, control) {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        lock(&state.database)?.verify_directory(directory)?;
        verify_child(&directory.path, &path)?;
        std::fs::remove_file(&path)
            .map_err(|cleanup| format!("{error}；清理未通过校验的临时文件失败：{cleanup}"))?;
        return Err(error);
    }
    let _operation = lock(&state.directory_operation)?;
    control.check()?;
    publish_bundle(state, &current, directory, &media, &path)
}

fn prepare_staging(
    job: &DownloadJob,
    option: &DownloadOption,
    staging: &Path,
) -> Result<(), String> {
    let root = job
        .directory
        .as_deref()
        .ok_or("Missing download directory")?;
    let signature = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&job.title, option, job.auth, job.expected_duration))
                .map_err(|error| error.to_string())?
        )
    );
    let marker = staging.join(".shadow-download-plan");
    if staging.try_exists().map_err(|error| error.to_string())? {
        verify_child(root, staging)?;
        let previous = if marker.try_exists().map_err(|error| error.to_string())? {
            verify_child(root, &marker)?;
            Some(std::fs::read_to_string(&marker).map_err(|error| error.to_string())?)
        } else {
            None
        };
        if previous.as_deref() != Some(&signature) {
            // This is only the checked, private staging directory for this UUID.
            // A different format/account must never resume incompatible partial bytes.
            std::fs::remove_dir_all(staging)
                .map_err(|error| format!("重建任务临时目录失败：{error}"))?;
        }
    }
    create_child(root, staging)?;
    std::fs::write(marker, signature).map_err(|error| error.to_string())
}

fn verify_download(
    path: &Path,
    option: &DownloadOption,
    expected: Option<f64>,
    control: &DownloadControl,
) -> Result<(), String> {
    let mut cmd = toolchain::hidden_command(&toolchain::ffprobe_path());
    cmd.args([
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ])
    .arg(path);
    let output = crate::services::download_process::run_command(cmd, control, true, |_| Ok(()))?;
    if !output.status.success() {
        return Err(format!("无法校验下载文件：{}", output.stderr.trim()));
    }
    let info = media_probe::parse_output(output.stdout.as_bytes())?;
    validate_download_info(&info, option, expected)
}

fn validate_download_info(
    info: &media_probe::MediaInfo,
    option: &DownloadOption,
    expected: Option<f64>,
) -> Result<(), String> {
    if info.media_type != option.media_type {
        return Err("下载文件的音视频类型与所选格式不一致，未加入播放列表".into());
    }
    if option.requires_audio && !info.has_audio {
        return Err("下载文件缺少音轨，未加入播放列表".into());
    }
    if let Some(expected) = expected {
        let actual = info
            .duration_secs
            .ok_or("无法读取下载文件时长，未加入播放列表")?;
        if (expected - actual).abs() > duration_tolerance(expected) {
            return Err(format!("内容时长不符：应为 {:.0} 秒，实际 {:.0} 秒。可能是试看内容或下载不完整；请核对登录来源和观看权限后重新解析。", expected, actual));
        }
    }
    // A height without width can be a site quality tier (for example Bilibili
    // labels a cropped 854x356 film "480p"). Only compare actual dimension pairs.
    if let Some((width, height)) = option.width.zip(option.height) {
        let (actual_width, actual_height) = info
            .video_width
            .zip(info.video_height)
            .ok_or("无法读取下载视频的分辨率")?;
        if actual_width.abs_diff(width) > 2 || actual_height.abs_diff(height) > 2 {
            return Err(format!("视频分辨率不符：所选格式为 {width}×{height}，实际为 {actual_width}×{actual_height}，请重新解析"));
        }
    }
    Ok(())
}

pub(super) fn parse_progress(line: &str) -> Result<Option<DownloadProgress>, String> {
    let Some(json) = line.strip_prefix("__SHADOW_PROGRESS__") else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| format!("Invalid download progress: {error}"))?;
    let positive = |key: &str| {
        value
            .get(key)
            .and_then(|value| value.as_f64())
            .filter(|number| number.is_finite() && *number >= 0.0)
    };
    let total = positive("total_bytes")
        .or_else(|| positive("total_bytes_estimate"))
        .filter(|number| *number > 0.0);
    Ok(Some(DownloadProgress {
        percent: positive("downloaded_bytes")
            .zip(total)
            .map(|(bytes, total)| (bytes / total * 100.0).clamp(0.0, 100.0)),
        speed: positive("speed"),
        eta: positive("eta"),
    }))
}

fn staging_path(job: &DownloadJob) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(&job.id).map_err(|_| "Invalid download identity")?;
    let root = job
        .directory
        .as_ref()
        .ok_or("Download directory is missing")?;
    Ok(root.join("downloading").join(&job.id))
}

fn publication_paths(job: &DownloadJob) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let staging = staging_path(job)?;
    let root = job
        .directory
        .as_deref()
        .ok_or("Missing download directory")?;
    if job.publication.is_empty() {
        return Err("下载保存日志缺少文件清单，无法自动恢复".into());
    }
    let mut sources = std::collections::HashSet::new();
    let mut outputs = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for file in &job.publication {
        for name in [&file.source_name, &file.output_name] {
            let mut parts = Path::new(name).components();
            if !matches!(parts.next(), Some(std::path::Component::Normal(_)))
                || parts.next().is_some()
                || name
                    .chars()
                    .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
                || name.ends_with(['.', ' '])
            {
                return Err("下载保存日志包含无效文件名".into());
            }
        }
        if !sources.insert(file.source_name.to_lowercase())
            || !outputs.insert(file.output_name.to_lowercase())
        {
            return Err("下载保存日志包含重复文件".into());
        }
        paths.push((
            staging.join(&file.source_name),
            root.join(&file.output_name),
        ));
    }
    if !paths
        .iter()
        .any(|(_, output)| Some(output) == job.output_path.as_ref())
    {
        return Err("下载保存日志与媒体输出路径不一致".into());
    }
    verify_child(root, &staging)?;
    Ok(paths)
}

fn plan_publication(job: &mut DownloadJob, assets: &mut [MediaAsset]) -> Result<(), String> {
    let staging = staging_path(job)?;
    let root = job
        .directory
        .as_deref()
        .ok_or("Missing download directory")?;
    let playback = assets
        .iter()
        .find(|asset| asset.kind == AssetKind::Playback)
        .ok_or("Download has no playback file")?;
    let stem = playback
        .path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("Downloaded filename is not UTF-8")?
        .to_string();
    let names = std::fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|value| value.file_name().to_string_lossy().to_lowercase()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    // Reserve the whole basename so existing subtitles or another media extension
    // cannot accidentally become associated with the new download.
    let mut selected = None;
    for number in 0..10_000 {
        let candidate = if number == 0 {
            stem.clone()
        } else {
            format!("{stem} ({number})")
        };
        let key = candidate.to_lowercase();
        if !names
            .iter()
            .any(|name| name == &key || name.starts_with(&format!("{key}.")))
        {
            selected = Some(candidate);
            break;
        }
    }
    let selected = selected.ok_or("同名文件过多，无法生成下载文件名")?;
    job.publication.clear();
    for asset in assets {
        if asset.path.parent() != Some(staging.as_path()) {
            return Err("下载文件不在任务临时目录中".into());
        }
        verify_child(root, &asset.path)?;
        let source_name = asset
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Downloaded filename is not UTF-8")?
            .to_string();
        let suffix = source_name
            .strip_prefix(&stem)
            .filter(|value| value.starts_with('.'))
            .ok_or("下载字幕名称与媒体不一致")?;
        let output_name = format!("{selected}{suffix}");
        job.publication.push(DownloadFile {
            source_name,
            output_name: output_name.clone(),
            stamp: stamp(&asset.path)?,
        });
        asset.path = root.join(output_name);
        if asset.kind == AssetKind::Playback {
            job.output_path = Some(asset.path.clone());
        }
    }
    publication_paths(job)?;
    Ok(())
}

fn verify_child(root: &Path, path: &Path) -> Result<(), String> {
    let actual = dunce::canonicalize(path).map_err(|error| error.to_string())?;
    if actual != path || !contains_path(root, &actual)? {
        return Err(format!("下载路径包含目录外的链接：{}", path.display()));
    }
    Ok(())
}

fn create_child(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err("Invalid download subdirectory".into());
        }
        current.push(part);
        if !current.try_exists().map_err(|error| error.to_string())? {
            std::fs::create_dir(&current).map_err(|error| error.to_string())?;
        }
        verify_child(root, &current)?;
    }
    Ok(())
}

// Holds directory_operation across all file moves and SQLite publication.
fn publish_bundle(
    state: &AppState,
    job: &DownloadJob,
    directory: &DirectorySnapshot,
    media: &Media,
    path: &Path,
) -> Result<(), String> {
    let staging = staging_path(job)?;
    if !path.is_file() || dunce::simplified(path).parent() != Some(staging.as_path()) {
        return Err("Downloaded media is outside its task directory".into());
    }
    verify_child(&directory.path, path)?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("Downloaded filename is not UTF-8")?;
    let mut assets =
        media_assets::subtitle_assets(&media.id, &staging, stem, AssetSource::Download)?;
    assets.push(MediaAsset {
        id: uuid::Uuid::new_v4().to_string(),
        media_id: media.id.clone(),
        kind: AssetKind::Playback,
        path: path.into(),
        language: None,
        source: AssetSource::Download,
    });
    let mut database = lock(&state.database)?;
    database.verify_directory(directory)?;
    let mut publishing = database.download_job(&job.id)?;
    if publishing.attempt != job.attempt || publishing.phase != DownloadPhase::Verifying {
        return Err("下载任务已停止".into());
    }
    publishing.phase = DownloadPhase::Publishing;
    plan_publication(&mut publishing, &mut assets)?;
    let files = publication_paths(&publishing)?;
    database.save_download_job(&publishing)?;
    let commit = (|| {
        for (source, destination) in &files {
            move_file(source, destination)?;
        }
        // Completion, assets and playlist membership commit in the same transaction.
        database.publish_directory_download(directory, &media.id, &assets, &job.id, job.attempt)
    })();
    if let Err(error) = commit {
        let rollback = restore_publication(&publishing);
        let error = match rollback {
            Ok(()) => {
                publishing.phase = DownloadPhase::Failed;
                publishing.output_path = None;
                publishing.publication.clear();
                error
            }
            Err(recovery) => {
                publishing.phase = DownloadPhase::RecoveryRequired;
                format!("{error}; {recovery}")
            }
        };
        publishing.error = Some(error.clone());
        database
            .save_download_job(&publishing)
            .map_err(|save| format!("{error}; {save}"))?;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::media::MediaType;
    use crate::services::{
        directory_library, download_process, playback_controller::PlaybackController,
    };
    use std::sync::{atomic::AtomicU64, Mutex};

    struct Fixture {
        state: AppState,
        temp: tempfile::TempDir,
        a: PathBuf,
        b: PathBuf,
    }
    fn state(database: PersistenceManager) -> AppState {
        AppState {
            settings: Arc::new(Mutex::new(database.load_settings().unwrap())),
            database: Arc::new(Mutex::new(database)),
            playback: Arc::new(Mutex::new(PlaybackController::new())),
            downloads: Arc::new(Mutex::new(DownloadRegistry::default())),
            directory_operation: Arc::new(Mutex::new(())),
            transcription_submission: Arc::new(tokio::sync::Mutex::new(())),
            playlist_snapshot_version: Arc::new(AtomicU64::new(0)),
        }
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let a = temp.path().join("A");
            let b = temp.path().join("B");
            std::fs::create_dir(&a).unwrap();
            std::fs::create_dir(&b).unwrap();
            let state =
                state(PersistenceManager::open_path(&temp.path().join("tasks.sqlite3")).unwrap());
            directory_library::update_directory(&state, Some(a.clone())).unwrap();
            Self {
                temp,
                state,
                a: dunce::canonicalize(a).unwrap(),
                b: dunce::canonicalize(b).unwrap(),
            }
        }
        fn start(&self) -> (DownloadJob, DirectorySnapshot, Arc<DownloadControl>, Media) {
            let draft = DownloadJob::new("https://youtu.be/download-fixture".into()).unwrap();
            lock(&self.state.database)
                .unwrap()
                .save_download_job(&draft)
                .unwrap();
            self.attempt(&draft.id)
        }
        fn attempt(
            &self,
            id: &str,
        ) -> (DownloadJob, DirectorySnapshot, Arc<DownloadControl>, Media) {
            let (job, directory, control) =
                prepare_attempt(&self.state, id, &DownloadAction::Retry).unwrap();
            let draft = crate::application::remote_media(
                job.url.clone(),
                "测试下载".into(),
                "download-fixture".into(),
                MediaType::Audio,
            )
            .unwrap();
            let mut db = lock(&self.state.database).unwrap();
            let option = fixture_option();
            let media = db
                .resolve_download_media(id, job.attempt, &draft, vec![option.clone()], None)
                .unwrap();
            let mut job = db.download_job(id).unwrap();
            job.selection = Some(option.id);
            job.phase = DownloadPhase::Verifying;
            db.save_download_job(&job).unwrap();
            (job, directory, control, media)
        }
        fn restart(&mut self) {
            let database =
                PersistenceManager::open_path(&self.temp.path().join("tasks.sqlite3")).unwrap();
            recover(&database).unwrap();
            self.state = state(database);
        }
    }
    fn staged_file(job: &DownloadJob) -> PathBuf {
        let staging = staging_path(job).unwrap();
        create_child(job.directory.as_ref().unwrap(), &staging).unwrap();
        let file = staging.join("测试下载.mp3");
        std::fs::write(&file, b"completed fixture").unwrap();
        file
    }

    fn fixture_option() -> DownloadOption {
        DownloadOption {
            id: "audio:original".into(),
            media_type: MediaType::Audio,
            height: None,
            width: None,
            format_selector: "audio".into(),
            extract_audio: false,
            requires_audio: true,
            limited_duration: None,
            video_codec: None,
        }
    }

    #[test]
    fn clear_history_preserves_running_tasks_files_media_and_playback() {
        let fixture = Fixture::new();
        let (completed, directory, control, media) = fixture.start();
        let path = staged_file(&completed);
        publish_bundle(&fixture.state, &completed, &directory, &media, &path).unwrap();
        let saved = lock(&fixture.state.database)
            .unwrap()
            .download_job(&completed.id)
            .unwrap();
        let output = saved.output_path.as_ref().unwrap();
        let bytes = std::fs::read(output).unwrap();
        let media_before = lock(&fixture.state.database)
            .unwrap()
            .media(&media.id)
            .unwrap();
        let revision = lock(&fixture.state.database).unwrap().playlist_revision;
        let entry = lock(&fixture.state.database)
            .unwrap()
            .playlist()
            .unwrap()
            .remove(0);
        let session = lock(&fixture.state.playback)
            .unwrap()
            .begin(media_before.clone(), Some(entry.id.clone()))
            .unwrap();
        let partial = fixture.a.join("downloading").join("kept.part");
        std::fs::write(&partial, b"partial download").unwrap();
        let mut active = Vec::new();
        let mut waiting = None;
        for phase in [
            DownloadPhase::Resolving,
            DownloadPhase::AwaitingSelection,
            DownloadPhase::Downloading,
            DownloadPhase::Verifying,
            DownloadPhase::Publishing,
            DownloadPhase::Failed,
            DownloadPhase::Canceled,
            DownloadPhase::Interrupted,
            DownloadPhase::RecoveryRequired,
        ] {
            let mut job = DownloadJob::new("https://example.com/history".into()).unwrap();
            job.phase = phase;
            job.attempt = 1;
            lock(&fixture.state.database)
                .unwrap()
                .save_download_job(&job)
                .unwrap();
            let control = Arc::new(DownloadControl::default());
            lock(&fixture.state.downloads).unwrap().tasks.insert(
                job.id.clone(),
                RuntimeTask {
                    attempt: job.attempt,
                    control: control.clone(),
                    progress: None,
                    error: None,
                },
            );
            if phase.is_active() {
                active.push((job.id.clone(), control));
            }
            if phase == DownloadPhase::AwaitingSelection {
                waiting = Some(job);
            }
        }
        // A worker that failed to persist its terminal state is shown as recovery_required.
        let mut errored = DownloadJob::new("https://example.com/runtime-error".into()).unwrap();
        errored.phase = DownloadPhase::Downloading;
        lock(&fixture.state.database)
            .unwrap()
            .save_download_job(&errored)
            .unwrap();
        lock(&fixture.state.downloads).unwrap().tasks.insert(
            errored.id.clone(),
            RuntimeTask {
                attempt: errored.attempt,
                control: Arc::new(DownloadControl::default()),
                progress: None,
                error: Some("stopped worker".into()),
            },
        );
        let clearable = snapshot(&fixture.state)
            .unwrap()
            .jobs
            .iter()
            .filter(|job| !job.phase.is_active())
            .count();
        assert_eq!(clear_history(&fixture.state).unwrap(), clearable);
        let remaining = snapshot(&fixture.state).unwrap();
        assert_eq!(remaining.jobs.len(), 4);
        assert!(remaining.jobs.iter().all(|job| job.phase.is_active()));
        for (id, control) in active {
            assert!(remaining.jobs.iter().any(|job| job.id == id));
            let registry = lock(&fixture.state.downloads).unwrap();
            assert!(Arc::ptr_eq(&registry.tasks[&id].control, &control));
            control.check().unwrap();
        }
        control.check().unwrap();
        assert_eq!(std::fs::read(output).unwrap(), bytes);
        assert_eq!(std::fs::read(partial).unwrap(), b"partial download");
        assert!(lock(&fixture.state.playback).unwrap().matches(session));
        let database = lock(&fixture.state.database).unwrap();
        assert_eq!(database.media(&media.id).unwrap(), media_before);
        assert_eq!(database.playlist_revision, revision);
        assert_eq!(database.playlist().unwrap()[0].id, entry.id);
        drop(database);
        finish_attempt(
            &fixture.state,
            &waiting.unwrap(),
            Err("late completion".into()),
        )
        .unwrap();
        assert_eq!(clear_history(&fixture.state).unwrap(), 0);
        // Reopening SQLite cannot bring cleared task records back.
        let reopened =
            PersistenceManager::open_path(&fixture.temp.path().join("tasks.sqlite3")).unwrap();
        assert_eq!(reopened.download_jobs().unwrap().len(), 4);
        assert!(reopened.download_job(&completed.id).is_err());
    }

    #[test]
    fn clear_history_database_failure_rolls_back_all_records_and_runtime_state() {
        let fixture = Fixture::new();
        let mut jobs = Vec::new();
        for created_at in [1, 2] {
            let mut job = DownloadJob::new("https://example.com/history".into()).unwrap();
            job.created_at = created_at;
            job.phase = DownloadPhase::Failed;
            lock(&fixture.state.database)
                .unwrap()
                .save_download_job(&job)
                .unwrap();
            lock(&fixture.state.downloads).unwrap().tasks.insert(
                job.id.clone(),
                RuntimeTask {
                    attempt: job.attempt,
                    control: Arc::new(DownloadControl::default()),
                    progress: None,
                    error: None,
                },
            );
            jobs.push(job);
        }
        lock(&fixture.state.database).unwrap().connection.execute_batch(
            "CREATE TRIGGER refuse_history_clear BEFORE DELETE ON download_jobs WHEN json_extract(OLD.data, '$.created_at') = 1 BEGIN SELECT RAISE(ABORT, 'injected history failure'); END;"
        ).unwrap();
        assert!(clear_history(&fixture.state)
            .unwrap_err()
            .contains("injected history failure"));
        assert_eq!(snapshot(&fixture.state).unwrap().jobs.len(), 2);
        assert_eq!(lock(&fixture.state.downloads).unwrap().tasks.len(), 2);
        for job in jobs {
            assert!(lock(&fixture.state.database)
                .unwrap()
                .download_job(&job.id)
                .is_ok());
        }
    }

    #[test]
    fn resolution_selection_persists_and_does_not_silently_change_after_restart() {
        let mut fixture = Fixture::new();
        let draft = DownloadJob::new("https://youtu.be/choices".into()).unwrap();
        let option = DownloadOption {
            id: "video:1080".into(),
            media_type: MediaType::Video,
            height: Some(1080),
            format_selector: "137+140".into(),
            ..fixture_option()
        };
        let media = crate::application::remote_media(
            draft.url.clone(),
            "Film - Episode 1".into(),
            "choices".into(),
            MediaType::Video,
        )
        .unwrap();
        let job = {
            let mut db = lock(&fixture.state.database).unwrap();
            db.save_download_job(&draft).unwrap();
            let job = db
                .start_download_attempt(&draft.id, &DownloadAction::Resolve(DownloadAuth::Edge))
                .unwrap();
            db.resolve_download_media(
                &job.id,
                job.attempt,
                &media,
                vec![option.clone()],
                Some(60.0),
            )
            .unwrap();
            let job = db.download_job(&job.id).unwrap();
            assert_eq!(job.phase, DownloadPhase::AwaitingSelection);
            assert!(db.playlist().unwrap().is_empty());
            job
        };
        fixture.restart();
        directory_library::update_directory(&fixture.state, Some(fixture.b.clone())).unwrap();
        let mut db = lock(&fixture.state.database).unwrap();
        let restored = db.download_job(&job.id).unwrap();
        assert_eq!(restored.phase, DownloadPhase::AwaitingSelection);
        assert_eq!(restored.auth, DownloadAuth::Edge);
        assert_eq!(restored.options, vec![option.clone()]);
        assert!(db
            .start_download_attempt(
                &job.id,
                &DownloadAction::Select {
                    option_id: "video:720".into(),
                    attempt: job.attempt
                }
            )
            .is_err());
        assert!(db
            .start_download_attempt(
                &job.id,
                &DownloadAction::Select {
                    option_id: option.id.clone(),
                    attempt: job.attempt + 1
                }
            )
            .is_err());
        let started = db
            .start_download_attempt(
                &job.id,
                &DownloadAction::Select {
                    option_id: option.id,
                    attempt: job.attempt,
                },
            )
            .unwrap();
        assert_eq!(started.directory.as_ref(), Some(&fixture.b));
        assert_eq!(started.selection.as_deref(), Some("video:1080"));
        let available = DownloadOption {
            id: "video:480".into(),
            height: Some(480),
            format_selector: "32".into(),
            ..option
        };
        db.resolve_download_media(
            &job.id,
            started.attempt,
            &media,
            vec![available],
            Some(60.0),
        )
        .unwrap();
        let refreshed = db.download_job(&job.id).unwrap();
        assert_eq!(refreshed.phase, DownloadPhase::AwaitingSelection);
        assert!(refreshed.selection.is_none());
        assert!(refreshed.error.is_some());
        assert!(db.playlist().unwrap().is_empty());
    }

    #[test]
    fn verification_rejects_audio_disguised_as_video_wrong_resolution_and_preview_duration() {
        let option = DownloadOption {
            id: "video:480".into(),
            height: Some(480),
            width: Some(640),
            media_type: MediaType::Video,
            ..fixture_option()
        };
        let audio = media_probe::parse_output(br#"{"format":{"duration":"360"},"streams":[{"codec_type":"audio","codec_name":"mp3"}]}"#).unwrap();
        assert!(validate_download_info(&audio, &option, Some(7403.0))
            .unwrap_err()
            .contains("类型"));
        let mut info = media_probe::parse_output(br#"{"format":{"duration":"360"},"streams":[{"codec_type":"video","codec_name":"h264","height":480,"width":640},{"codec_type":"audio","codec_name":"aac"}]}"#).unwrap();
        assert!(validate_download_info(&info, &option, Some(7403.0))
            .unwrap_err()
            .contains("时长"));
        info.duration_secs = Some(7403.0);
        assert!(validate_download_info(&info, &option, Some(7403.0)).is_ok());
        info.video_height = Some(360);
        assert!(validate_download_info(&info, &option, Some(7403.0))
            .unwrap_err()
            .contains("分辨率"));
        let film = media_probe::parse_output(br#"{"format":{"duration":"360.151"},"streams":[{"codec_type":"video","codec_name":"h264","width":854,"height":356},{"codec_type":"audio","codec_name":"aac"}]}"#).unwrap();
        let tier = DownloadOption {
            width: None,
            ..option
        };
        assert!(validate_download_info(&film, &tier, Some(360.151)).is_ok());
        assert!(validate_download_info(&film, &tier, Some(7403.0))
            .unwrap_err()
            .contains("时长"));
        let cover_audio = media_probe::parse_output(br#"{"format":{"duration":"60"},"streams":[{"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},{"codec_type":"audio","codec_name":"mp3"}]}"#).unwrap();
        assert_eq!(cover_audio.media_type, MediaType::Audio);
        assert!(validate_download_info(&cover_audio, &fixture_option(), Some(60.0)).is_ok());
    }

    #[test]
    fn publishing_updates_an_existing_medias_title_and_type_atomically() {
        let fixture = Fixture::new();
        let (mut job, directory, _, media) = fixture.start();
        job.title = "时空恋旅人 - 正片".into();
        let option = DownloadOption {
            id: "video:480".into(),
            media_type: MediaType::Video,
            height: Some(480),
            ..fixture_option()
        };
        job.selection = Some(option.id.clone());
        job.options = vec![option];
        lock(&fixture.state.database)
            .unwrap()
            .save_download_job(&job)
            .unwrap();
        let path = staged_file(&job);
        lock(&fixture.state.database).unwrap().connection.execute_batch("CREATE TRIGGER fail_media_publication BEFORE INSERT ON playlist_entries BEGIN SELECT RAISE(ABORT, 'publication fixture'); END;").unwrap();
        assert!(publish_bundle(&fixture.state, &job, &directory, &media, &path).is_err());
        {
            let db = lock(&fixture.state.database).unwrap();
            assert_eq!(db.media(&media.id).unwrap().title, media.title);
            assert_eq!(db.media(&media.id).unwrap().media_type, MediaType::Audio);
            db.connection
                .execute_batch("DROP TRIGGER fail_media_publication;")
                .unwrap();
            job.phase = DownloadPhase::Verifying;
            db.save_download_job(&job).unwrap();
        }
        publish_bundle(&fixture.state, &job, &directory, &media, &path).unwrap();
        let db = lock(&fixture.state.database).unwrap();
        assert_eq!(db.media(&media.id).unwrap().title, "时空恋旅人 - 正片");
        assert_eq!(db.media(&media.id).unwrap().media_type, MediaType::Video);
        assert_eq!(
            db.download_job(&job.id).unwrap().phase,
            DownloadPhase::Completed
        );
    }

    #[test]
    fn changed_format_discards_only_private_incompatible_partials() {
        let fixture = Fixture::new();
        let (job, _, _, _) = fixture.start();
        let staging = staging_path(&job).unwrap();
        let original = fixture_option();
        prepare_staging(&job, &original, &staging).unwrap();
        let partial = staging.join("track.mp3.part");
        std::fs::write(&partial, b"partial bytes").unwrap();
        let kept = fixture.a.join("keep.mp3");
        std::fs::write(&kept, b"existing library file").unwrap();
        prepare_staging(&job, &original, &staging).unwrap();
        assert!(partial.is_file());
        let changed = DownloadOption {
            format_selector: "other".into(),
            ..original
        };
        prepare_staging(&job, &changed, &staging).unwrap();
        assert!(!partial.exists());
        assert!(kept.is_file());
    }

    #[test]
    #[ignore = "requires bundled yt-dlp and FFmpeg; serves generated 360p/720p HLS on localhost"]
    fn real_tools_download_the_selected_resolution_and_verify_streams() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("output");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        for (height, width) in [(360, 640), (720, 1280)] {
            let result = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
                .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
                .arg(format!("color=c=blue:s={width}x{height}:r=10"))
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100",
                    "-t",
                    "2",
                    "-c:v",
                    "libx264",
                    "-threads",
                    "1",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-f",
                    "hls",
                    "-hls_time",
                    "1",
                    "-hls_list_size",
                    "0",
                ])
                .arg(source.join(format!("{height}.m3u8")))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::fs::write(source.join("master.m3u8"), "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360,CODECS=\"avc1.64001f,mp4a.40.2\"\n360.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1280x720,CODECS=\"avc1.64001f,mp4a.40.2\"\n720.m3u8\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/master.m3u8", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        struct Server {
            stop: Arc<AtomicBool>,
            worker: Option<std::thread::JoinHandle<()>>,
        }
        impl Drop for Server {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                self.worker.take().unwrap().join().unwrap();
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let _server = Server {
            stop,
            worker: Some(std::thread::spawn(move || {
                while !running.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut header = [0; 8192];
                            let Ok(count) = stream.read(&mut header) else {
                                continue;
                            };
                            let request = String::from_utf8_lossy(&header[..count]);
                            let name = request
                                .split_whitespace()
                                .nth(1)
                                .unwrap_or("/")
                                .trim_start_matches('/');
                            if name.contains("..") {
                                continue;
                            }
                            let Ok(bytes) = std::fs::read(source.join(name)) else {
                                continue;
                            };
                            let content_type = if name.ends_with(".m3u8") {
                                "application/vnd.apple.mpegurl"
                            } else {
                                "video/mp2t"
                            };
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                            if stream.write_all(response.as_bytes()).is_ok()
                                && !request.starts_with("HEAD ")
                            {
                                let _ = stream.write_all(&bytes);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => panic!("fixture server: {error}"),
                    }
                }
            })),
        };
        let control = DownloadControl::default();
        let metadata =
            OnlineResolver::resolve_metadata(&url, DownloadAuth::Public, &control).unwrap();
        let options = metadata.download_options().unwrap();
        assert_eq!(
            options
                .iter()
                .filter_map(|option| option.height)
                .collect::<Vec<_>>(),
            [720, 360]
        );
        let selected = options
            .iter()
            .find(|option| option.id == "video:360")
            .unwrap();
        let path = OnlineResolver::download_media(
            &DownloadRequest {
                url: &url,
                title: "分辨率测试 100%",
                output_dir: &destination,
                option: selected,
                auth: DownloadAuth::Public,
                extra_subtitle_lang: None,
            },
            &control,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(path.file_name().unwrap(), "分辨率测试 100%.mp4");
        verify_download(&path, selected, Some(2.0), &control).unwrap();
        assert!(verify_download(&path, selected, Some(7403.0), &control)
            .unwrap_err()
            .contains("时长"));
    }

    #[test]
    fn restart_and_retry_use_current_directory_and_reject_old_attempt_results() {
        let mut fixture = Fixture::new();
        let (old_job, old_directory, _, _) = fixture.start();
        let partial = staged_file(&old_job);
        fixture.restart();
        assert_eq!(
            lock(&fixture.state.database)
                .unwrap()
                .download_job(&old_job.id)
                .unwrap()
                .phase,
            DownloadPhase::Interrupted
        );
        std::fs::write(fixture.b.join("playing.wav"), b"local fixture").unwrap();
        directory_library::update_directory(&fixture.state, Some(fixture.b.clone())).unwrap();
        let playing = {
            let db = lock(&fixture.state.database).unwrap();
            let entry = db.playlist().unwrap().remove(0);
            (db.entry_media(&entry.id).unwrap(), entry.id)
        };
        let session = lock(&fixture.state.playback)
            .unwrap()
            .begin(playing.0, Some(playing.1))
            .unwrap();
        let (job, directory, _, media) = fixture.attempt(&old_job.id);
        assert_eq!(directory.path, fixture.b);
        assert_eq!(job.directory.as_ref(), Some(&fixture.b));
        assert_eq!(job.attempt, old_job.attempt + 1);
        assert!(!lock(&fixture.state.database)
            .unwrap()
            .finish_download_attempt(
                &old_job.id,
                old_job.attempt,
                DownloadPhase::Failed,
                Some("late failure".into()),
                None
            )
            .unwrap());
        assert!(lock(&fixture.state.database)
            .unwrap()
            .verify_directory(&old_directory)
            .is_err());
        publish_bundle(&fixture.state, &job, &directory, &media, &staged_file(&job)).unwrap();
        let db = lock(&fixture.state.database).unwrap();
        let finished = db.download_job(&job.id).unwrap();
        assert_eq!(finished.phase, DownloadPhase::Completed);
        assert!(finished.output_path.unwrap().starts_with(&fixture.b));
        assert_eq!(db.playlist().unwrap().len(), 2);
        assert!(partial.is_file());
        assert_eq!(std::fs::read(partial).unwrap(), b"completed fixture");
        drop(db);
        assert!(lock(&fixture.state.playback).unwrap().matches(session));
    }

    #[test]
    fn directory_switch_stops_writers_before_committing_the_new_directory() {
        let fixture = Fixture::new();
        let (job, _, control, _) = fixture.start();
        let marker = fixture.a.join("downloading-writer-marker");
        let worker = download_process::tests::start_fixture(control, &marker);
        std::fs::write(fixture.b.join("new.wav"), b"local fixture").unwrap();
        directory_library::update_directory(&fixture.state, Some(fixture.b.clone())).unwrap();
        assert!(worker.join().unwrap().is_err());
        download_process::tests::assert_file_stopped(&marker);
        let db = lock(&fixture.state.database).unwrap();
        let interrupted = db.download_job(&job.id).unwrap();
        assert_eq!(interrupted.phase, DownloadPhase::Interrupted);
        assert_eq!(
            interrupted.interrupt_reason.as_deref(),
            Some("directory_changed")
        );
        assert_eq!(db.directory().unwrap().path, fixture.b);
        assert_eq!(db.playlist().unwrap().len(), 1);
        assert!(contains_path(
            &fixture.b,
            db.entry_media(&db.playlist().unwrap()[0].id)
                .unwrap()
                .local_path()
                .unwrap()
        )
        .unwrap());
    }

    #[test]
    fn publication_failure_rolls_back_files_assets_playlist_and_completion() {
        let fixture = Fixture::new();
        let (job, directory, _, media) = fixture.start();
        let file = staged_file(&job);
        let subtitle = file.with_extension("zh.srt");
        std::fs::write(&subtitle, b"rollback subtitle").unwrap();
        lock(&fixture.state.database).unwrap().connection.execute_batch("CREATE TRIGGER fail_download BEFORE INSERT ON playlist_entries BEGIN SELECT RAISE(ABORT, 'publication fixture'); END;").unwrap();
        assert!(
            publish_bundle(&fixture.state, &job, &directory, &media, &file)
                .unwrap_err()
                .contains("publication fixture")
        );
        let db = lock(&fixture.state.database).unwrap();
        assert!(db.playlist().unwrap().is_empty());
        assert!(db.media(&media.id).unwrap().assets.is_empty());
        assert_eq!(
            db.download_job(&job.id).unwrap().phase,
            DownloadPhase::Failed
        );
        assert!(file.is_file());
        assert_eq!(std::fs::read(&subtitle).unwrap(), b"rollback subtitle");
        assert!(!fixture.a.join(subtitle.file_name().unwrap()).exists());
        assert!(!fixture.a.join(file.file_name().unwrap()).exists());
        db.connection
            .execute_batch("DROP TRIGGER fail_download;")
            .unwrap();
        drop(db);
        let (next, directory, _, media) = fixture.attempt(&job.id);
        assert_eq!(staging_path(&next).unwrap(), file.parent().unwrap());
        publish_bundle(&fixture.state, &next, &directory, &media, &file).unwrap();
        assert_eq!(
            lock(&fixture.state.database)
                .unwrap()
                .download_job(&job.id)
                .unwrap()
                .phase,
            DownloadPhase::Completed
        );
    }

    #[test]
    fn restart_restores_a_rename_that_was_not_committed_to_sqlite() {
        // A crash can happen before, between or after media/subtitle moves.
        for moved in 0..=2 {
            let mut fixture = Fixture::new();
            let (mut job, _, _, media) = fixture.start();
            let file = staged_file(&job);
            let subtitle = file.with_extension("zh.srt");
            std::fs::write(&subtitle, b"subtitle fixture").unwrap();
            let mut assets = publication_assets(&media.id, &file);
            job.phase = DownloadPhase::Publishing;
            plan_publication(&mut job, &mut assets).unwrap();
            lock(&fixture.state.database)
                .unwrap()
                .save_download_job(&job)
                .unwrap();
            let paths = publication_paths(&job).unwrap();
            for (source, destination) in paths.iter().take(moved) {
                move_file(source, destination).unwrap();
            }
            fixture.restart();
            assert_eq!(std::fs::read(&file).unwrap(), b"completed fixture");
            assert_eq!(std::fs::read(&subtitle).unwrap(), b"subtitle fixture");
            assert!(paths.iter().all(|(_, path)| !path.exists()));
            let db = lock(&fixture.state.database).unwrap();
            let restored = db.download_job(&job.id).unwrap();
            assert_eq!(restored.phase, DownloadPhase::Interrupted);
            assert!(restored.publication.is_empty());
            assert!(db.playlist().unwrap().is_empty());
        }
    }

    fn publication_assets(media_id: &str, file: &Path) -> Vec<MediaAsset> {
        let mut assets = media_assets::subtitle_assets(
            media_id,
            file.parent().unwrap(),
            file.file_stem().unwrap().to_str().unwrap(),
            AssetSource::Download,
        )
        .unwrap();
        assets.push(MediaAsset {
            id: uuid::Uuid::new_v4().to_string(),
            media_id: media_id.into(),
            kind: AssetKind::Playback,
            path: file.into(),
            language: None,
            source: AssetSource::Download,
        });
        assets
    }

    #[test]
    fn publication_is_flat_and_renames_the_whole_basename_without_overwriting() {
        let fixture = Fixture::new();
        let (job, directory, _, media) = fixture.start();
        let file = staged_file(&job);
        std::fs::write(file.with_extension("zh.srt"), b"new subtitles").unwrap();
        let marker = file.parent().unwrap().join(".shadow-download-plan");
        std::fs::write(&marker, b"private plan").unwrap();
        let original = fixture.a.join("测试下载.mp3");
        let original_subtitle = fixture.a.join("测试下载.zh.srt");
        std::fs::write(&original, b"old media").unwrap();
        std::fs::write(&original_subtitle, b"old subtitles").unwrap();
        publish_bundle(&fixture.state, &job, &directory, &media, &file).unwrap();
        let db = lock(&fixture.state.database).unwrap();
        let saved = db.download_job(&job.id).unwrap();
        assert_eq!(saved.output_path, Some(fixture.a.join("测试下载 (1).mp3")));
        assert_eq!(
            std::fs::read(fixture.a.join("测试下载 (1).zh.srt")).unwrap(),
            b"new subtitles"
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"old media");
        assert_eq!(std::fs::read(&original_subtitle).unwrap(), b"old subtitles");
        assert!(marker.exists());
        assert!(!fixture.a.join(".shadow-download-plan").exists());
        assert!(!fixture.a.join("media").exists());
        assert!(saved.publication.is_empty());
        assert!(db
            .media(&media.id)
            .unwrap()
            .assets
            .iter()
            .all(|asset| asset.path.parent() == Some(fixture.a.as_path())));
        drop(db);
        directory_library::update_directory(&fixture.state, None).unwrap();
        let db = lock(&fixture.state.database).unwrap();
        assert_eq!(db.playlist().unwrap().len(), 2);
        assert_eq!(
            db.media(&media.id).unwrap().local_path(),
            saved.output_path.as_deref()
        );
    }

    #[test]
    fn publication_recovery_preserves_foreign_files_and_rejects_changed_outputs() {
        let fixture = Fixture::new();
        let (mut job, _, _, media) = fixture.start();
        let file = staged_file(&job);
        let mut assets = publication_assets(&media.id, &file);
        plan_publication(&mut job, &mut assets).unwrap();
        let output = job.output_path.clone().unwrap();
        // A file created after choosing the basename must not be overwritten.
        std::fs::write(&output, b"foreign file").unwrap();
        assert!(move_file(&file, &output).is_err());
        restore_publication(&job).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"foreign file");
        std::fs::remove_file(&output).unwrap();
        move_file(&file, &output).unwrap();
        std::fs::write(&output, b"edited after crash").unwrap();
        assert!(restore_publication(&job)
            .unwrap_err()
            .contains("文件已被修改"));
        assert_eq!(std::fs::read(&output).unwrap(), b"edited after crash");
        job.publication[0].output_name = "../outside.mp3".into();
        assert!(publication_paths(&job).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn publication_file_move_failure_restores_subtitles_before_returning() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        let (job, directory, _, media) = fixture.start();
        let file = staged_file(&job);
        let subtitle = file.with_extension("zh.srt");
        std::fs::write(&subtitle, b"subtitle before locked media").unwrap();
        // Permit reads and writes, but not the second file's rename/delete.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&file)
            .unwrap();
        assert!(publish_bundle(&fixture.state, &job, &directory, &media, &file).is_err());
        assert_eq!(
            std::fs::read(&subtitle).unwrap(),
            b"subtitle before locked media"
        );
        assert!(!fixture.a.join(subtitle.file_name().unwrap()).exists());
        assert!(!fixture.a.join(file.file_name().unwrap()).exists());
        let db = lock(&fixture.state.database).unwrap();
        let saved = db.download_job(&job.id).unwrap();
        assert_eq!(saved.phase, DownloadPhase::Failed);
        assert!(saved.publication.is_empty());
        assert!(saved.output_path.is_none());
        assert!(db.playlist().unwrap().is_empty());
        drop(db);
        drop(held);
    }

    #[test]
    fn progress_handles_known_and_unknown_download_sizes() {
        let known = parse_progress(
            r#"__SHADOW_PROGRESS__{"downloaded_bytes":40,"total_bytes":80,"speed":1024,"eta":3}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(known.percent, Some(50.0));
        let unknown = parse_progress(
            r#"__SHADOW_PROGRESS__{"downloaded_bytes":40,"total_bytes":null,"eta":null}"#,
        )
        .unwrap()
        .unwrap();
        assert!(unknown.percent.is_none());
        assert!(unknown.eta.is_none());
        assert!(parse_progress("[Merger] Merging formats")
            .unwrap()
            .is_none());
        assert!(parse_progress("__SHADOW_PROGRESS__invalid").is_err());
    }

    #[test]
    fn task_database_has_one_application_owner_and_unlocks_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("shadow-player.lock");
        let first = crate::services::persistence::acquire_instance_lock(&path).unwrap();
        assert!(crate::services::persistence::acquire_instance_lock(&path).is_err());
        drop(first);
        assert!(crate::services::persistence::acquire_instance_lock(&path).is_ok());
    }

    #[test]
    fn failed_directory_commit_preserves_the_directory_and_reports_interruption() {
        let fixture = Fixture::new();
        let (job, directory, control, _) = fixture.start();
        lock(&fixture.state.database).unwrap().connection.execute_batch(
            "CREATE TRIGGER fail_directory BEFORE UPDATE ON download_directory BEGIN SELECT RAISE(ABORT, 'directory fixture'); END;"
        ).unwrap();
        let error = directory_library::update_directory(&fixture.state, Some(fixture.b.clone()))
            .err()
            .unwrap();
        assert!(error.contains("directory fixture"));
        assert!(error.contains("进行中的下载已中断"));
        assert!(control.check().is_err());
        let db = lock(&fixture.state.database).unwrap();
        assert_eq!(db.directory().unwrap(), directory);
        assert_eq!(
            db.download_job(&job.id).unwrap().phase,
            DownloadPhase::Interrupted
        );
        assert_eq!(
            db.load_settings().unwrap().download_directory,
            fixture.a.to_str().unwrap()
        );
    }
}
