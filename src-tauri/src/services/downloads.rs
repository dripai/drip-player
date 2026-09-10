use crate::app_state::{lock, AppState};
use crate::models::download::{DownloadJob, DownloadPhase, DownloadProgress, DownloadSnapshot};
use crate::models::media::{AssetKind, AssetSource, Media, MediaAsset, MediaOrigin};
use crate::services::directory_library::{contains_path, DirectorySnapshot};
use crate::services::download_process::DownloadControl;
use crate::services::online_resolver::{DownloadRequest, OnlineResolver};
use crate::services::{media_assets, persistence::PersistenceManager};
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

// Recover the rename journal before startup scanning or a manual retry.
fn restore_publication(job: &DownloadJob) -> Result<(), String> {
    let (staging, destination) = bundle_paths(job)?;
    let output = job
        .output_path
        .as_ref()
        .ok_or("Download publication has no output path")?;
    if output.parent() != Some(destination.as_path()) {
        return Err("Invalid download publication path".into());
    }
    if destination
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        if staging.try_exists().map_err(|error| error.to_string())? {
            return Err("下载恢复失败：临时目录和目标目录同时存在".into());
        }
        verify_child(
            job.directory
                .as_deref()
                .ok_or("Missing download directory")?,
            &destination,
        )?;
        std::fs::rename(&destination, &staging)
            .map_err(|error| format!("恢复未提交的下载失败：{error}"))?;
    } else if !staging.try_exists().map_err(|error| error.to_string())? {
        return Err("下载恢复失败：临时文件和目标文件均不存在".into());
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
            database.save_download_job(&job)?;
        }
    }
    Ok(())
}

fn prepare_attempt(
    state: &AppState,
    id: &str,
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
    let job = database.start_download_attempt(id)?;
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

pub fn submit(state: &AppState, app: &AppHandle, url: String) -> Result<String, String> {
    let url = url.trim().to_string();
    crate::models::media::provider_key_for_url(&url)?;
    let _operation = lock(&state.directory_operation)?;
    {
        let database = lock(&state.database)?;
        if let Some(job) = database
            .download_jobs()?
            .iter()
            .find(|job| job.url == url && job.phase.is_active())
        {
            return Ok(job.id.clone());
        }
    }
    let job = DownloadJob::new(url)?;
    lock(&state.database)?.save_download_job(&job)?;
    let prepared = prepare_attempt(state, &job.id)?;
    launch(state.clone(), app.clone(), prepared);
    Ok(job.id)
}

pub fn retry(state: &AppState, app: &AppHandle, id: &str) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let prepared = prepare_attempt(state, id)?;
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
        let finish = (|| {
            let _operation = lock(&state.directory_operation)?;
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
                    database.finish_download_attempt(
                        &job.id,
                        job.attempt,
                        phase,
                        Some(error),
                        None,
                    )?;
                }
            }
            let mut registry = lock(&state.downloads)?;
            if registry
                .tasks
                .get(&job.id)
                .is_some_and(|task| task.attempt == job.attempt)
            {
                registry.tasks.remove(&job.id);
            }
            Ok::<_, String>(())
        })();
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
    let job = database.download_job(id)?;
    database.finish_download_attempt(id, job.attempt, DownloadPhase::Canceled, None, None)?;
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
    let metadata = OnlineResolver::resolve_metadata(&job.url, control)?;
    let kind = metadata.get_media_type();
    let draft =
        crate::application::remote_media(job.url.clone(), metadata.title, metadata.id, kind)?;
    let (media, current) = {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        let mut database = lock(&state.database)?;
        database.verify_directory(directory)?;
        let media = database.resolve_download_media(&job.id, job.attempt, &draft)?;
        (media, database.download_job(&job.id)?)
    };
    notify(app);
    // Reuse only a completed file that actually belongs to the current directory.
    if let Some(path) = media.cached_path().filter(|path| path.is_file()) {
        let path = dunce::canonicalize(path).map_err(|error| error.to_string())?;
        if contains_path(&directory.path, &path)? {
            let _operation = lock(&state.directory_operation)?;
            control.check()?;
            let assets = media
                .assets
                .iter()
                .filter(|asset| asset.path.is_file())
                .cloned()
                .collect::<Vec<_>>();
            lock(&state.database)?.publish_directory_download(
                directory,
                &media.id,
                &assets,
                &job.id,
                job.attempt,
            )?;
            return Ok(());
        }
    }
    let (staging, _) = bundle_paths(&current)?;
    {
        let _operation = lock(&state.directory_operation)?;
        control.check()?;
        lock(&state.database)?.verify_directory(directory)?;
        create_child(&directory.path, &staging)?;
    }
    if !matches!(media.origin, MediaOrigin::Remote { .. }) {
        return Err("Expected remote media".into());
    }
    let path = OnlineResolver::download_media(
        &DownloadRequest {
            url: &job.url,
            title: &media.title,
            output_dir: &staging,
            media_type: &media.media_type,
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
    let _operation = lock(&state.directory_operation)?;
    control.check()?;
    publish_bundle(state, &current, directory, &media, &path)
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

fn bundle_paths(job: &DownloadJob) -> Result<(PathBuf, PathBuf), String> {
    uuid::Uuid::parse_str(&job.id).map_err(|_| "Invalid download identity")?;
    let media_id = job
        .media_id
        .as_deref()
        .ok_or("Download media is unresolved")?;
    uuid::Uuid::parse_str(media_id).map_err(|_| "Invalid media identity")?;
    let root = job
        .directory
        .as_ref()
        .ok_or("Download directory is missing")?;
    Ok((
        root.join("downloading").join(&job.id),
        root.join("media").join(media_id).join(&job.id),
    ))
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

// Holds directory_operation across the filesystem rename and SQLite publication.
fn publish_bundle(
    state: &AppState,
    job: &DownloadJob,
    directory: &DirectorySnapshot,
    media: &Media,
    path: &Path,
) -> Result<(), String> {
    let (staging, destination) = bundle_paths(job)?;
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
    for asset in &mut assets {
        asset.path = destination.join(asset.path.file_name().ok_or("Missing download filename")?);
    }
    create_child(
        &directory.path,
        destination.parent().ok_or("Missing bundle parent")?,
    )?;
    let mut database = lock(&state.database)?;
    database.verify_directory(directory)?;
    let mut publishing = database.download_job(&job.id)?;
    if publishing.attempt != job.attempt || publishing.phase != DownloadPhase::Downloading {
        return Err("下载任务已停止".into());
    }
    publishing.phase = DownloadPhase::Publishing;
    publishing.output_path = assets
        .iter()
        .find(|asset| asset.kind == AssetKind::Playback)
        .map(|asset| asset.path.clone());
    if destination
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("下载目标目录已存在，不能覆盖".into());
    }
    database.save_download_job(&publishing)?;
    let mut moved = false;
    let commit = (|| {
        std::fs::rename(&staging, &destination)
            .map_err(|error| format!("保存下载文件失败：{error}"))?;
        moved = true;
        // Completion, assets and playlist membership commit in the same transaction.
        database.publish_directory_download(directory, &media.id, &assets, &job.id, job.attempt)
    })();
    if let Err(error) = commit {
        let rollback = if moved {
            restore_publication(&publishing)
        } else {
            Ok(())
        };
        let error = match rollback {
            Ok(()) => {
                publishing.phase = DownloadPhase::Failed;
                publishing.output_path = None;
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
            let (job, directory, control) = prepare_attempt(&self.state, id).unwrap();
            let draft = crate::application::remote_media(
                job.url.clone(),
                "测试下载".into(),
                "download-fixture".into(),
                MediaType::Audio,
            )
            .unwrap();
            let mut db = lock(&self.state.database).unwrap();
            let media = db.resolve_download_media(id, job.attempt, &draft).unwrap();
            (db.download_job(id).unwrap(), directory, control, media)
        }
        fn restart(&mut self) {
            let database =
                PersistenceManager::open_path(&self.temp.path().join("tasks.sqlite3")).unwrap();
            recover(&database).unwrap();
            self.state = state(database);
        }
    }
    fn staged_file(job: &DownloadJob) -> PathBuf {
        let (staging, _) = bundle_paths(job).unwrap();
        create_child(job.directory.as_ref().unwrap(), &staging).unwrap();
        let file = staging.join("测试下载.mp3");
        std::fs::write(&file, b"completed fixture").unwrap();
        file
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
        assert!(!bundle_paths(&job).unwrap().1.exists());
        db.connection
            .execute_batch("DROP TRIGGER fail_download;")
            .unwrap();
        drop(db);
        let (next, directory, _, media) = fixture.attempt(&job.id);
        assert_eq!(bundle_paths(&next).unwrap().0, file.parent().unwrap());
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
        let mut fixture = Fixture::new();
        let (mut job, _, _, _) = fixture.start();
        let file = staged_file(&job);
        let (staging, destination) = bundle_paths(&job).unwrap();
        create_child(&fixture.a, destination.parent().unwrap()).unwrap();
        job.phase = DownloadPhase::Publishing;
        job.output_path = Some(destination.join(file.file_name().unwrap()));
        lock(&fixture.state.database)
            .unwrap()
            .save_download_job(&job)
            .unwrap();
        std::fs::rename(&staging, &destination).unwrap();
        fixture.restart();
        assert!(file.is_file());
        assert!(!destination.exists());
        let db = lock(&fixture.state.database).unwrap();
        assert_eq!(
            db.download_job(&job.id).unwrap().phase,
            DownloadPhase::Interrupted
        );
        assert!(db.playlist().unwrap().is_empty());
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
