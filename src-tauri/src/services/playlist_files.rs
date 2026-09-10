use crate::app_state::{lock, AppState};
use crate::models::media::{canonical_local_identity, AssetKind, Media, MediaOrigin};
use crate::services::directory_library::{contains_path, DirectorySnapshot};
use crate::services::download_store::write_job;
use crate::services::file_operations::{move_file, stamp};
use crate::services::persistence::{
    advance_revision, check_revision, database_error, path_text, PersistenceManager,
};
use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// The confirmation is bound to this exact entry, directory and file version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistFile {
    pub item_id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub stamp: String,
    pub directory_revision: u64,
    pub stem: String,
    pub extension: String,
}

fn checked_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取文件 {}：{error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("不是普通媒体文件：{}", path.display()));
    }
    let path = dunce::canonicalize(path).map_err(|error| error.to_string())?;
    if !contains_path(root, &path)? {
        return Err("文件不在当前保存目录中，请刷新播放列表".into());
    }
    Ok(path)
}

fn inspect(database: &PersistenceManager, item_id: &str) -> Result<(PlaylistFile, Media), String> {
    let directory = database.directory()?;
    let root = dunce::canonicalize(&directory.path).map_err(|error| error.to_string())?;
    let media = database.entry_media(item_id)?;
    let path = checked_path(&root, media.local_path().ok_or("媒体尚未下载")?)?;
    let file = PlaylistFile {
        item_id: item_id.into(),
        media_id: media.id.clone(),
        stamp: stamp(&path)?,
        directory_revision: directory.revision,
        stem: path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("文件名不是 UTF-8")?
            .into(),
        extension: path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or("文件缺少扩展名")?
            .into(),
        path,
    };
    Ok((file, media))
}

pub fn get_file(state: &AppState, item_id: &str) -> Result<PlaylistFile, String> {
    let _operation = lock(&state.directory_operation)?;
    let database = lock(&state.database)?;
    Ok(inspect(&database, item_id)?.0)
}

fn validate(database: &PersistenceManager, target: &PlaylistFile) -> Result<Media, String> {
    let (current, media) = inspect(database, &target.item_id)?;
    if current != *target {
        return Err("文件或保存目录已变化，请关闭弹窗并重新操作".into());
    }
    if database.download_jobs()?.iter().any(|job| {
        job.media_id.as_deref() == Some(&media.id)
            && (job.phase.is_active()
                || job.phase == crate::models::download::DownloadPhase::RecoveryRequired)
    }) {
        return Err("此媒体有进行中或待恢复的下载任务，请先处理下载任务".into());
    }
    Ok(media)
}

fn validate_name(name: &str, extension: &str) -> Result<(), String> {
    if name.is_empty()
        || name != name.trim()
        || name.ends_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err("名称不能为空，不能包含路径、特殊字符或首尾空格".into());
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    ) || (base.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && matches!(base.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("不能使用系统保留的文件名".into());
    }
    let filename = format!("{name}.{extension}");
    let length = if cfg!(windows) {
        filename.encode_utf16().count()
    } else {
        filename.len()
    };
    if length > 255 {
        return Err("文件名过长".into());
    }
    Ok(())
}

#[derive(Clone)]
struct FileMove {
    from: PathBuf,
    to: PathBuf,
}

fn rollback(moved: &[FileMove], error: String) -> String {
    let mut errors = vec![error];
    for item in moved.iter().rev() {
        if let Err(error) = move_file(&item.to, &item.from) {
            errors.push(format!("恢复原文件名失败：{error}"));
        }
    }
    errors.join("；")
}

fn rename_plan(
    media: &Media,
    target: &PlaylistFile,
    name: &str,
    directory: &DirectorySnapshot,
) -> Result<Vec<FileMove>, String> {
    validate_name(name, &target.extension)?;
    let destination = target
        .path
        .with_file_name(format!("{name}.{}", target.extension));
    let mut files = vec![FileMove {
        from: target.path.clone(),
        to: destination,
    }];
    let prefix = format!("{}.", target.stem);
    for asset in media
        .assets
        .iter()
        .filter(|a| a.kind == AssetKind::Subtitle)
    {
        let path = checked_path(&directory.path, &asset.path)?;
        if path.parent() != target.path.parent() {
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("字幕文件名不是 UTF-8")?;
        if let Some(suffix) = filename.strip_prefix(&prefix) {
            validate_name(name, suffix)?;
            files.push(FileMove {
                to: path.with_file_name(format!("{name}.{suffix}")),
                from: path,
            });
        }
    }
    let mut plan = Vec::new();
    for file in files {
        if file.from == file.to {
            continue;
        }
        let case_only = cfg!(windows)
            && path_text(&file.from)?.to_lowercase() == path_text(&file.to)?.to_lowercase();
        if case_only {
            let temporary = file
                .from
                .with_file_name(format!(".shadow-rename-{}", uuid::Uuid::new_v4()));
            plan.push(FileMove {
                from: file.from,
                to: temporary.clone(),
            });
            plan.push(FileMove {
                from: temporary,
                to: file.to,
            });
        } else {
            if file.to.try_exists().map_err(|error| error.to_string())? {
                return Err(format!("同名文件已存在：{}", file.to.display()));
            }
            plan.push(file);
        }
    }
    Ok(plan)
}

impl PersistenceManager {
    fn rename_file(
        &mut self,
        target: &PlaylistFile,
        media: &Media,
        name: &str,
        plan: &[FileMove],
    ) -> Result<(), String> {
        let jobs = self.download_jobs()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        let mut moved = Vec::new();
        for item in plan {
            if let Err(error) = move_file(&item.from, &item.to) {
                return Err(rollback(&moved, error));
            }
            moved.push(item.clone());
        }
        let result = (|| {
            let destination = target
                .path
                .with_file_name(format!("{name}.{}", target.extension));
            if matches!(media.origin, MediaOrigin::Local { .. }) {
                let (path, key) = canonical_local_identity(&destination)?;
                tx.execute("UPDATE media SET title = ?1, local_path = ?2, canonical_key = ?3 WHERE id = ?4", params![name, path_text(&path)?, key, media.id]).map_err(database_error)?;
            } else {
                tx.execute(
                    "UPDATE media SET title = ?1 WHERE id = ?2",
                    params![name, media.id],
                )
                .map_err(database_error)?;
            }
            // Apply the same moves (including case-only intermediates) to every stored path.
            for item in plan {
                for asset in &media.assets {
                    let stored: String = tx
                        .query_row(
                            "SELECT path FROM media_assets WHERE id = ?1",
                            [&asset.id],
                            |r| r.get(0),
                        )
                        .map_err(database_error)?;
                    if dunce::simplified(Path::new(&stored)) == item.from {
                        tx.execute(
                            "UPDATE media_assets SET path = ?1 WHERE id = ?2",
                            params![path_text(&item.to)?, asset.id],
                        )
                        .map_err(database_error)?;
                    }
                }
            }
            for mut job in jobs {
                if job
                    .output_path
                    .as_deref()
                    .is_some_and(|p| dunce::simplified(p) == target.path)
                {
                    job.output_path = Some(destination.clone());
                    job.title = name.into();
                    write_job(&tx, &job)?;
                }
            }
            let next = advance_revision(&tx, self.playlist_revision)?;
            tx.commit().map_err(database_error)?;
            Ok(next)
        })();
        match result {
            Ok(next) => {
                self.playlist_revision = next;
                Ok(())
            }
            Err(error) => Err(rollback(&moved, error)),
        }
    }

    fn delete_file(&mut self, target: &PlaylistFile) -> Result<(), String> {
        let jobs = self.download_jobs()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        tx.execute(
            "DELETE FROM playlist_entries WHERE media_id = ?1",
            [&target.media_id],
        )
        .map_err(database_error)?;
        tx.execute(
            "DELETE FROM media_assets WHERE media_id = ?1 AND kind = 'playback'",
            [&target.media_id],
        )
        .map_err(database_error)?;
        // Keep historical learning references, but release the local-path identity
        // so a different file later placed at this name cannot inherit those notes.
        tx.execute(
            "UPDATE media SET canonical_key = 'deleted-local:' || id WHERE id = ?1 AND origin_kind = 'local'",
            [&target.media_id],
        ).map_err(database_error)?;
        for mut job in jobs {
            if job
                .output_path
                .as_deref()
                .is_some_and(|p| dunce::simplified(p) == target.path)
            {
                job.output_path = None;
                write_job(&tx, &job)?;
            }
        }
        let next = advance_revision(&tx, self.playlist_revision)?;
        // SQL errors occur before deletion. A disk/permission error rolls SQL back.
        std::fs::remove_file(&target.path)
            .map_err(|error| format!("无法删除文件 {}：{error}", target.path.display()))?;
        tx.commit().map_err(|error| {
            format!(
                "文件已永久删除，但数据库提交失败：{error}。请重启并刷新播放列表：{}",
                target.path.display()
            )
        })?;
        self.playlist_revision = next;
        Ok(())
    }
}

pub fn rename(state: &AppState, target: &PlaylistFile, name: &str) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let mut playback = lock(&state.playback)?;
    let mut database = lock(&state.database)?;
    let media = validate(&database, target)?;
    let plan = rename_plan(&media, target, name, &database.directory()?)?;
    if plan.is_empty() {
        return Ok(());
    }
    if playback
        .session
        .as_ref()
        .is_some_and(|s| s.media.id == media.id)
    {
        playback.stop()?;
    }
    database.rename_file(target, &media, name, &plan)
}

pub fn delete(state: &AppState, target: &PlaylistFile) -> Result<(), String> {
    let _operation = lock(&state.directory_operation)?;
    let mut playback = lock(&state.playback)?;
    let mut database = lock(&state.database)?;
    let media = validate(&database, target)?;
    if playback
        .session
        .as_ref()
        .is_some_and(|s| s.media.id == media.id)
    {
        playback.stop()?;
    }
    database.delete_file(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        download::{DownloadJob, DownloadPhase},
        media::{AssetSource, MediaAsset},
    };
    use crate::services::{
        directory_library::{scan_directory, update_directory},
        downloads::DownloadRegistry,
        playback_controller::PlaybackController,
    };
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    fn fixture() -> (tempfile::TempDir, AppState, PlaylistFile) {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("media");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("原片.mp4"), b"test video").unwrap();
        std::fs::write(folder.join("原片.en.srt"), b"test subtitle").unwrap();
        let mut db = PersistenceManager::open_path(&temp.path().join("test.sqlite3")).unwrap();
        let scan = scan_directory(&folder).unwrap();
        let previous = db.directory().unwrap();
        let mut settings = db.load_settings().unwrap();
        settings.revision += 1;
        settings.download_directory = path_text(&scan.path).unwrap().into();
        db.replace_directory_playlist(&scan, &previous, Some(&settings))
            .unwrap();
        let item_id = db.playlist().unwrap()[0].id.clone();
        let state = AppState {
            database: Arc::new(Mutex::new(db)),
            settings: Arc::new(Mutex::new(settings)),
            playback: Arc::new(Mutex::new(PlaybackController::new())),
            downloads: Arc::new(Mutex::new(DownloadRegistry::default())),
            directory_operation: Arc::new(Mutex::new(())),
            transcription_submission: Arc::new(tokio::sync::Mutex::new(())),
            playlist_snapshot_version: Arc::new(AtomicU64::new(0)),
        };
        let target = get_file(&state, &item_id).unwrap();
        (temp, state, target)
    }

    fn download_record(state: &AppState, target: &PlaylistFile) -> String {
        let mut db = lock(&state.database).unwrap();
        db.connection.execute("UPDATE media SET origin_kind = 'remote', local_path = NULL, canonical_key = 'remote:test:1', remote_url = 'https://example.com/video', provider = 'test', external_id = '1' WHERE id = ?1", [&target.media_id]).unwrap();
        db.attach_assets(
            &target.media_id,
            &[MediaAsset {
                id: uuid::Uuid::new_v4().to_string(),
                media_id: target.media_id.clone(),
                path: target.path.clone(),
                kind: AssetKind::Playback,
                source: AssetSource::Download,
                language: None,
            }],
        )
        .unwrap();
        let mut job = DownloadJob::new("https://example.com/video".into()).unwrap();
        job.media_id = Some(target.media_id.clone());
        job.title = target.stem.clone();
        job.phase = DownloadPhase::Completed;
        job.output_path = Some(target.path.clone());
        db.save_download_job(&job).unwrap();
        job.id
    }

    #[test]
    fn rename_preserves_identity_subtitles_learning_and_refresh_membership() {
        let (_temp, state, target) = fixture();
        {
            let db = lock(&state.database).unwrap();
            db.connection.execute("INSERT INTO transcripts (id, media_id, label, data) VALUES ('transcript', ?1, 'subtitles', '{}')", [&target.media_id]).unwrap();
            db.connection.execute("INSERT INTO learning_cue_notes (transcript_id, cue_id, favorite) VALUES ('transcript', 0, 1)", []).unwrap();
        }
        let media = lock(&state.database)
            .unwrap()
            .media(&target.media_id)
            .unwrap();
        let session = lock(&state.playback)
            .unwrap()
            .begin(media, Some(target.item_id.clone()))
            .unwrap();
        rename(&state, &target, "新的片名").unwrap();
        assert!(!lock(&state.playback).unwrap().is_requested(session));
        assert!(!target.path.exists());
        assert!(!target.path.with_file_name("原片.en.srt").exists());
        let renamed = get_file(&state, &target.item_id).unwrap();
        assert_eq!(renamed.media_id, target.media_id);
        assert_eq!(renamed.path.file_name().unwrap(), "新的片名.mp4");
        let db = lock(&state.database).unwrap();
        let media = db.media(&target.media_id).unwrap();
        assert_eq!(media.title, "新的片名");
        assert!(media.assets.iter().all(|a| a.path.is_file()));
        assert_eq!(
            db.connection
                .query_row("SELECT favorite FROM learning_cue_notes", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(db);
        update_directory(&state, None).unwrap();
        assert_eq!(get_file(&state, &target.item_id).unwrap(), renamed);
        assert_eq!(lock(&state.database).unwrap().playlist().unwrap().len(), 1);
    }

    #[test]
    fn download_rename_and_delete_keep_all_recorded_paths_consistent() {
        let (_temp, state, target) = fixture();
        let job_id = download_record(&state, &target);
        rename(&state, &target, "完整版").unwrap();
        let renamed = get_file(&state, &target.item_id).unwrap();
        {
            let db = lock(&state.database).unwrap();
            let job = db.download_job(&job_id).unwrap();
            assert_eq!(job.title, "完整版");
            assert_eq!(job.output_path, Some(renamed.path.clone()));
            assert_eq!(
                db.media(&target.media_id).unwrap().cached_path(),
                Some(renamed.path.as_path())
            );
        }
        update_directory(&state, None).unwrap();
        assert_eq!(get_file(&state, &target.item_id).unwrap(), renamed);
        delete(&state, &renamed).unwrap();
        assert!(!renamed.path.exists());
        assert!(renamed.path.with_file_name("完整版.en.srt").exists());
        let db = lock(&state.database).unwrap();
        assert!(db.playlist().unwrap().is_empty());
        assert!(db.download_job(&job_id).unwrap().output_path.is_none());
        assert!(db.media(&target.media_id).unwrap().cached_path().is_none());
        drop(db);
        update_directory(&state, None).unwrap();
        assert!(lock(&state.database)
            .unwrap()
            .playlist()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn conflicts_invalid_names_and_stale_confirmations_never_change_files() {
        let (temp, state, target) = fixture();
        let occupied = target.path.with_file_name("已存在.mp4");
        std::fs::write(&occupied, b"keep me").unwrap();
        assert!(rename(&state, &target, "已存在").is_err());
        for name in [
            "",
            "../elsewhere",
            "CON",
            "LPT1",
            "bad:name",
            "尾部.",
            " 空格",
        ] {
            assert!(rename(&state, &target, name).is_err(), "{name}");
        }
        assert_eq!(std::fs::read(&occupied).unwrap(), b"keep me");
        let mut forged = target.clone();
        forged.path = occupied;
        assert!(delete(&state, &forged).is_err());
        std::fs::write(&target.path, b"replaced by another file").unwrap();
        assert!(delete(&state, &target).is_err());
        let fresh = get_file(&state, &target.item_id).unwrap();
        let new_directory = temp.path().join("other");
        std::fs::create_dir(&new_directory).unwrap();
        update_directory(&state, Some(new_directory)).unwrap();
        assert!(rename(&state, &fresh, "new").is_err());
        assert!(delete(&state, &fresh).is_err());
        assert!(target.path.is_file());
    }

    #[test]
    fn database_failures_restore_renamed_files_and_do_not_delete_files() {
        let (_temp, state, target) = fixture();
        lock(&state.database).unwrap().connection.execute_batch("CREATE TRIGGER fail_revision BEFORE UPDATE ON playlist_state BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
        assert!(rename(&state, &target, "不能保存")
            .unwrap_err()
            .contains("injected failure"));
        assert_eq!(get_file(&state, &target.item_id).unwrap(), target);
        assert!(target.path.with_file_name("原片.en.srt").exists());
        assert!(!target.path.with_file_name("不能保存.mp4").exists());
        assert!(!target.path.with_file_name("不能保存.en.srt").exists());
        assert!(delete(&state, &target)
            .unwrap_err()
            .contains("injected failure"));
        assert_eq!(get_file(&state, &target.item_id).unwrap(), target);
    }

    #[test]
    fn active_download_blocks_file_mutations() {
        let (_temp, state, target) = fixture();
        let job_id = download_record(&state, &target);
        let db = lock(&state.database).unwrap();
        let mut job = db.download_job(&job_id).unwrap();
        job.phase = DownloadPhase::Downloading;
        db.save_download_job(&job).unwrap();
        drop(db);
        assert!(rename(&state, &target, "new").is_err());
        assert!(delete(&state, &target).is_err());
        assert!(target.path.exists());
    }

    #[test]
    fn deleting_local_file_preserves_learning_without_reusing_its_identity() {
        let (_temp, state, target) = fixture();
        lock(&state.database).unwrap().connection.execute(
            "INSERT INTO transcripts (id, media_id, label, data) VALUES ('saved', ?1, 'notes', '{}')",
            [&target.media_id],
        ).unwrap();
        delete(&state, &target).unwrap();
        assert!(target.path.with_file_name("原片.en.srt").exists());
        std::fs::write(&target.path, b"a different video with the same filename").unwrap();
        update_directory(&state, None).unwrap();
        let db = lock(&state.database).unwrap();
        let replacement = db.playlist().unwrap().remove(0);
        assert_ne!(replacement.media_id, target.media_id);
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT media_id FROM transcripts WHERE id = 'saved'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            target.media_id
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_only_rename_and_locked_file_preserve_data() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_temp, state, target) = fixture();
        let subtitle_lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(target.path.with_file_name("原片.en.srt"))
            .unwrap();
        assert!(rename(&state, &target, "BlockedSubtitle").is_err());
        assert_eq!(get_file(&state, &target.item_id).unwrap(), target);
        assert!(!target.path.with_file_name("BlockedSubtitle.mp4").exists());
        drop(subtitle_lock);
        rename(&state, &target, "Clip").unwrap();
        let upper = get_file(&state, &target.item_id).unwrap();
        rename(&state, &upper, "clip").unwrap();
        let lower = get_file(&state, &target.item_id).unwrap();
        assert_eq!(lower.stem, "clip");
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&lower.path)
            .unwrap();
        assert!(delete(&state, &lower).is_err());
        assert!(rename(&state, &lower, "Locked").is_err());
        assert_eq!(get_file(&state, &target.item_id).unwrap(), lower);
        drop(handle);
        delete(&state, &lower).unwrap();
        assert!(!lower.path.exists());
    }
}
