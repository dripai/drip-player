use crate::app_state::{lock, AppState};
use crate::models::media::{AssetSource, Media, MediaAsset};
use crate::models::settings::{AppSettings, AppSettingsPatch};
use crate::services::persistence::{
    advance_revision, check_revision, database_error, path_text, put_assets, put_media, read_media,
    write_settings, PersistenceManager,
};
use crate::services::{media_assets, media_capabilities};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectorySnapshot {
    pub path: PathBuf,
    pub revision: u64,
}

pub struct DirectoryScan {
    pub path: PathBuf,
    pub media: Vec<Media>,
}

pub struct DirectoryUpdate {
    pub settings: AppSettings,
    pub playback_error: Option<String>,
}

pub(super) fn initialize(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(include_str!("directory_schema.sql"))
        .map_err(database_error)?;
    let initialized: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM download_directory WHERE id = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if !initialized {
        let path = media_assets::cache_root()?;
        std::fs::create_dir_all(&path).map_err(|error| {
            format!(
                "Cannot create download directory {}: {error}",
                path.display()
            )
        })?;
        let path = dunce::canonicalize(path).map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO download_directory (id, path, revision) VALUES (1, ?1, 0)",
            [path_text(&path)?],
        )
        .map_err(database_error)?;
    }
    Ok(())
}

fn read_directory(connection: &Connection) -> Result<DirectorySnapshot, String> {
    connection
        .query_row(
            "SELECT path, revision FROM download_directory WHERE id = 1",
            [],
            |row| {
                Ok(DirectorySnapshot {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    revision: row.get(1)?,
                })
            },
        )
        .map_err(database_error)
}

fn path_key(path: &Path) -> Result<String, String> {
    let text = path_text(dunce::simplified(path))?.replace('\\', "/");
    Ok(if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    })
}

// Both arguments must already be canonical paths, or generated descendants of one.
pub fn contains_path(root: &Path, path: &Path) -> Result<bool, String> {
    Ok(path_key(path)?.starts_with(&format!("{}/", path_key(root)?.trim_end_matches('/'))))
}

pub fn scan_directory(path: &Path) -> Result<DirectoryScan, String> {
    if !path.is_absolute() {
        return Err("请选择绝对目录路径".into());
    }
    let root = dunce::canonicalize(path)
        .map_err(|error| format!("无法读取目录 {}：{error}", path.display()))?;
    if !root.is_dir() {
        return Err(format!("不是文件夹：{}", root.display()));
    }
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        if !visited.insert(path_key(&directory)?) {
            continue;
        }
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("无法读取目录 {}：{error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("无法读取目录项：{error}"))?;
            let path = entry.path();
            // This is the application's unpublished download area, never playlist content.
            if directory == root
                && matches!(entry.file_name().to_str(), Some("downloading" | "remux"))
            {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("无法读取文件类型 {}：{error}", path.display()))?;
            if kind.is_symlink() {
                continue;
            }
            let canonical = dunce::canonicalize(&path)
                .map_err(|error| format!("无法读取路径 {}：{error}", path.display()))?;
            if !contains_path(&root, &canonical)? {
                continue;
            }
            if kind.is_dir() {
                pending.push(canonical);
            } else if kind.is_file() && media_capabilities::is_supported_media_path(&canonical) {
                files.push(canonical);
            }
        }
    }
    files.sort();
    files.dedup();
    let media = files
        .iter()
        .map(|path| media_assets::local_media(path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DirectoryScan { path: root, media })
}

impl PersistenceManager {
    pub fn directory(&self) -> Result<DirectorySnapshot, String> {
        read_directory(&self.connection)
    }

    pub fn verify_directory(&self, expected: &DirectorySnapshot) -> Result<(), String> {
        if self.directory()? != *expected {
            return Err("保存目录已切换，请重试当前操作".into());
        }
        Ok(())
    }

    pub fn replace_directory_playlist(
        &mut self,
        scan: &DirectoryScan,
        expected: &DirectorySnapshot,
        settings: Option<&AppSettings>,
    ) -> Result<HashSet<String>, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        if read_directory(&tx)? != *expected {
            return Err("保存目录已切换，请重试刷新".into());
        }
        let old: HashMap<String, (String, u64)> = tx
            .prepare("SELECT media_id, id, added_at FROM playlist_entries")
            .map_err(database_error)?
            .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
            .map_err(database_error)?
            .collect::<Result<_, _>>()
            .map_err(database_error)?;
        let playback_assets: Vec<(String, String)> = tx
            .prepare("SELECT media_id, path FROM media_assets WHERE kind = 'playback'")
            .map_err(database_error)?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(database_error)?
            .collect::<Result<_, _>>()
            .map_err(database_error)?;
        let mut owners = HashMap::new();
        for (id, path) in playback_assets {
            owners.insert(path_key(Path::new(&path))?, id);
        }
        let mut ids = Vec::new();
        let mut selected = HashSet::new();
        for candidate in &scan.media {
            let path = candidate.local_path().ok_or("Expected a local file")?;
            if !contains_path(&scan.path, path)? {
                return Err("扫描结果包含目录外的文件".into());
            }
            if !std::fs::symlink_metadata(path)
                .map_err(|error| format!("扫描后文件已变更 {}：{error}", path.display()))?
                .is_file()
            {
                return Err(format!("扫描后文件已变更：{}", path.display()));
            }
            let id = if let Some(id) = owners.get(&path_key(path)?) {
                // Reuse a downloaded resource's identity instead of adding it as a duplicate local file.
                let mut subtitles = candidate.assets.clone();
                for asset in &mut subtitles {
                    asset.media_id = id.clone();
                    asset.source = AssetSource::Download;
                }
                tx.execute(
                    "DELETE FROM media_assets WHERE media_id = ?1 AND kind = 'subtitle'",
                    [id],
                )
                .map_err(database_error)?;
                put_assets(&tx, id, &subtitles)?;
                id.clone()
            } else {
                let id = put_media(&tx, candidate)?;
                tx.execute(
                    "DELETE FROM media_assets WHERE media_id = ?1 AND kind = 'subtitle'",
                    [&id],
                )
                .map_err(database_error)?;
                put_assets(&tx, &id, &candidate.assets)?;
                id
            };
            if selected.insert(id.clone()) {
                ids.push(id);
            }
        }
        tx.execute("DELETE FROM playlist_entries", [])
            .map_err(database_error)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();
        let mut entries = HashSet::new();
        for (position, media_id) in ids.iter().enumerate() {
            let (id, added_at) = old
                .get(media_id)
                .cloned()
                .unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), now));
            tx.execute("INSERT INTO playlist_entries (id, media_id, position, added_at) VALUES (?1, ?2, ?3, ?4)", params![id, media_id, position, added_at]).map_err(database_error)?;
            entries.insert(id);
        }
        if let Some(settings) = settings {
            if settings.download_directory != path_text(&scan.path)? {
                return Err("目录与设置不一致".into());
            }
            write_settings(&tx, settings)?;
            let revision = expected
                .revision
                .checked_add(1)
                .ok_or("Directory revision overflow")?;
            tx.execute(
                "UPDATE download_directory SET path = ?1, revision = ?2 WHERE id = 1",
                params![settings.download_directory, revision],
            )
            .map_err(database_error)?;
        } else if scan.path != expected.path {
            return Err("刷新目录与当前设置不一致".into());
        }
        let revision = advance_revision(&tx, self.playlist_revision)?;
        tx.commit().map_err(database_error)?;
        self.playlist_revision = revision;
        Ok(entries)
    }

    pub fn publish_directory_download(
        &mut self,
        directory: &DirectorySnapshot,
        media_id: &str,
        assets: &[MediaAsset],
        job_id: &str,
        attempt: u64,
    ) -> Result<(), String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        if read_directory(&tx)? != *directory {
            return Err("保存目录已切换，本次下载未加入播放列表".into());
        }
        let mut job = crate::services::download_store::active_job(&tx, job_id, attempt)?;
        if job.phase != crate::models::download::DownloadPhase::Publishing {
            return Err("下载文件尚未通过校验并进入发布阶段".into());
        }
        if job.media_id.as_deref() != Some(media_id)
            || job.directory.as_ref() != Some(&directory.path)
        {
            return Err("下载任务与文件目录不一致".into());
        }
        for asset in assets {
            if !contains_path(&directory.path, &asset.path)? {
                return Err("下载文件不在当前保存目录中".into());
            }
        }
        tx.execute(
            "DELETE FROM media_assets WHERE media_id = ?1 AND source = 'download'",
            [media_id],
        )
        .map_err(database_error)?;
        let option = job
            .options
            .iter()
            .find(|option| Some(&option.id) == job.selection.as_ref())
            .ok_or("下载任务缺少已校验的格式选择")?;
        let kind = match option.media_type {
            crate::models::media::MediaType::Audio => "Audio",
            crate::models::media::MediaType::Video => "Video",
        };
        tx.execute(
            "UPDATE media SET title = ?1, media_type = ?2 WHERE id = ?3",
            params![job.title, kind, media_id],
        )
        .map_err(database_error)?;
        put_assets(&tx, media_id, assets)?;
        read_media(&tx, media_id)?;
        let entry: Option<String> = tx
            .query_row(
                "SELECT id FROM playlist_entries WHERE media_id = ?1",
                [media_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if entry.is_none() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs();
            tx.execute("INSERT INTO playlist_entries (id, media_id, position, added_at) SELECT ?1, ?2, COALESCE(MAX(position), -1) + 1, ?3 FROM playlist_entries", params![uuid::Uuid::new_v4().to_string(), media_id, now]).map_err(database_error)?;
        }
        job.output_path = Some(
            assets
                .iter()
                .find(|asset| asset.kind == crate::models::media::AssetKind::Playback)
                .ok_or("Download has no playback file")?
                .path
                .clone(),
        );
        job.phase = crate::models::download::DownloadPhase::Completed;
        job.publication.clear();
        job.error = None;
        crate::services::download_store::write_job(&tx, &job)?;
        let revision = advance_revision(&tx, self.playlist_revision)?;
        tx.commit().map_err(database_error)?;
        self.playlist_revision = revision;
        Ok(())
    }
}

pub fn update_directory(
    state: &AppState,
    requested: Option<PathBuf>,
) -> Result<DirectoryUpdate, String> {
    let _operation = lock(&state.directory_operation)?;
    crate::services::downloads::recover_publications(&*lock(&state.database)?)?;
    let previous = lock(&state.database)?.directory()?;
    let scan = scan_directory(requested.as_deref().unwrap_or(&previous.path))?;
    if requested.is_some() {
        tempfile::Builder::new()
            .prefix(".shadow-write-")
            .tempfile_in(&scan.path)
            .map_err(|error| format!("目录不可写 {}：{error}", scan.path.display()))?
            .close()
            .map_err(|error| format!("无法清理目录写入检查文件：{error}"))?;
    }
    let interrupted_downloads = requested.is_some()
        && lock(&state.database)?
            .download_jobs()?
            .iter()
            .any(|job| job.phase.is_active());
    if requested.is_some() {
        crate::services::downloads::interrupt_all(state, "directory_changed")?;
    }
    // Lock order: directory operation -> settings -> playback -> database -> downloads.
    let mut current = lock(&state.settings)?;
    let mut playback = lock(&state.playback)?;
    let mut next = current.patched(AppSettingsPatch::default())?;
    next.download_directory = path_text(&scan.path)?.into();
    let entries = lock(&state.database)?
        .replace_directory_playlist(&scan, &previous, requested.as_ref().map(|_| &next))
        .map_err(|error| {
            if interrupted_downloads {
                format!("{error}；目录和列表未更改，进行中的下载已中断")
            } else {
                error
            }
        })?;
    if requested.is_some() {
        *current = next;
    }
    let stop = requested.is_some()
        || playback.session.as_ref().is_some_and(|session| {
            session
                .playlist_entry_id
                .as_ref()
                .is_none_or(|id| !entries.contains(id))
        });
    let playback_error = if stop {
        playback
            .stop()
            .err()
            .map(|error| format!("目录和播放列表已更新，但停止旧播放失败：{error}"))
    } else {
        None
    };
    Ok(DirectoryUpdate {
        settings: current.clone(),
        playback_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::media::{AssetKind, MediaType};
    use crate::services::{downloads::DownloadRegistry, playback_controller::PlaybackController};
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    fn media_file(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"directory scan fixture").unwrap();
        dunce::canonicalize(path).unwrap()
    }

    fn choose(database: &mut PersistenceManager, path: &Path) -> DirectorySnapshot {
        let scan = scan_directory(path).unwrap();
        let previous = database.directory().unwrap();
        let mut next = database
            .load_settings()
            .unwrap()
            .patched(AppSettingsPatch::default())
            .unwrap();
        next.download_directory = path_text(&scan.path).unwrap().into();
        database
            .replace_directory_playlist(&scan, &previous, Some(&next))
            .unwrap();
        database.directory().unwrap()
    }

    #[test]
    fn directory_switch_replaces_membership_and_empty_directory_clears_it() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        let old_file = media_file(&a, "old.mp3");
        let new_file = media_file(&b, "nested/new.wav");
        media_file(&b, "new.srt");
        media_file(&b, "downloading/job/partial.mp3");
        media_file(&b, "remux/generated.mp4");
        let mut db = PersistenceManager::open_path(&temp.path().join("library.sqlite3")).unwrap();
        choose(&mut db, &a);
        let previous_entry = db.playlist().unwrap()[0].id.clone();
        let directory = choose(&mut db, &b);
        let entries = db.playlist().unwrap();
        assert_eq!(entries.len(), 1);
        assert_ne!(entries[0].id, previous_entry);
        assert_eq!(
            dunce::simplified(
                db.media(&entries[0].media_id)
                    .unwrap()
                    .local_path()
                    .unwrap()
            ),
            new_file
        );
        assert_eq!(
            db.load_settings().unwrap().download_directory,
            path_text(&directory.path).unwrap()
        );
        assert!(old_file.is_file());
        let first = entries[0].id.clone();
        db.replace_directory_playlist(&scan_directory(&b).unwrap(), &directory, None)
            .unwrap();
        assert_eq!(db.playlist().unwrap()[0].id, first);
        std::fs::remove_file(new_file).unwrap();
        db.replace_directory_playlist(&scan_directory(&b).unwrap(), &directory, None)
            .unwrap();
        assert!(db.playlist().unwrap().is_empty());
        let empty = temp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        choose(&mut db, &empty);
        assert!(db.playlist().unwrap().is_empty());
        assert!(old_file.is_file());
    }

    #[test]
    fn directory_transaction_failure_preserves_settings_playlist_and_media() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        media_file(&a, "old.mp3");
        media_file(&b, "new.mp3");
        let mut db = PersistenceManager::open_path(&temp.path().join("library.sqlite3")).unwrap();
        let previous = choose(&mut db, &a);
        let settings = db.load_settings().unwrap();
        let entry = db.playlist().unwrap()[0].id.clone();
        let scan = scan_directory(&b).unwrap();
        let mut next = settings.patched(AppSettingsPatch::default()).unwrap();
        next.download_directory = path_text(&scan.path).unwrap().into();
        db.connection.execute_batch("CREATE TRIGGER reject_directory BEFORE UPDATE ON download_directory BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(db
            .replace_directory_playlist(&scan, &previous, Some(&next))
            .unwrap_err()
            .contains("fixture failure"));
        assert_eq!(db.directory().unwrap(), previous);
        assert_eq!(db.load_settings().unwrap().revision, settings.revision);
        assert_eq!(db.playlist().unwrap()[0].id, entry);
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM media", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn directory_refresh_reuses_download_identity_and_rejects_old_completions() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut db = PersistenceManager::open_path(&temp.path().join("library.sqlite3")).unwrap();
        let directory = choose(&mut db, &a);
        let remote = crate::application::remote_media(
            "https://youtu.be/fixture".into(),
            "Downloaded song".into(),
            "fixture".into(),
            MediaType::Audio,
        )
        .unwrap();
        let remote = db.register_media(&remote).unwrap();
        let path = media_file(&a, "media/song.mp3");
        let assets = vec![MediaAsset {
            id: uuid::Uuid::new_v4().to_string(),
            media_id: remote.id.clone(),
            kind: AssetKind::Playback,
            path,
            language: None,
            source: AssetSource::Download,
        }];
        let mut job =
            crate::models::download::DownloadJob::new("https://youtu.be/fixture".into()).unwrap();
        job.media_id = Some(remote.id.clone());
        job.directory = Some(directory.path.clone());
        job.phase = crate::models::download::DownloadPhase::Publishing;
        job.title = remote.title.clone();
        job.selection = Some("audio:original".into());
        job.options = vec![crate::models::download::DownloadOption {
            video_codec: None,
            id: "audio:original".into(),
            media_type: MediaType::Audio,
            height: None,
            width: None,
            format_selector: "audio".into(),
            extract_audio: false,
            requires_audio: true,
            limited_duration: None,
        }];
        db.save_download_job(&job).unwrap();
        db.publish_directory_download(&directory, &remote.id, &assets, &job.id, job.attempt)
            .unwrap();
        let entry = db.playlist().unwrap()[0].id.clone();
        db.replace_directory_playlist(&scan_directory(&a).unwrap(), &directory, None)
            .unwrap();
        assert_eq!(db.playlist().unwrap().len(), 1);
        assert_eq!(db.playlist().unwrap()[0].media_id, remote.id);
        assert_eq!(db.playlist().unwrap()[0].id, entry);
        choose(&mut db, &b);
        assert!(db
            .publish_directory_download(&directory, &remote.id, &assets, &job.id, job.attempt)
            .is_err());
        assert!(db.playlist().unwrap().is_empty());
        choose(&mut db, &a);
        assert!(db
            .publish_directory_download(&directory, &remote.id, &assets, &job.id, job.attempt)
            .is_err());
        assert_eq!(db.playlist().unwrap().len(), 1);
    }

    #[test]
    fn directory_missing_or_changed_files_never_replace_the_committed_list() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        media_file(&a, "old.mp3");
        let file = media_file(&b, "new.mp3");
        let mut db = PersistenceManager::open_path(&temp.path().join("library.sqlite3")).unwrap();
        let directory = choose(&mut db, &a);
        assert!(scan_directory(&temp.path().join("missing")).is_err());
        assert!(scan_directory(&file).is_err());
        let scan = scan_directory(&b).unwrap();
        std::fs::remove_file(file).unwrap();
        let mut next = db
            .load_settings()
            .unwrap()
            .patched(AppSettingsPatch::default())
            .unwrap();
        next.download_directory = path_text(&scan.path).unwrap().into();
        assert!(db
            .replace_directory_playlist(&scan, &directory, Some(&next))
            .is_err());
        assert_eq!(db.directory().unwrap(), directory);
        assert_eq!(db.playlist().unwrap().len(), 1);
    }

    #[test]
    fn directory_switch_stops_old_playback_but_failed_scan_preserves_it() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("A");
        let b = temp.path().join("B");
        media_file(&a, "old.mp3");
        media_file(&b, "new.mp3");
        let mut database =
            PersistenceManager::open_path(&temp.path().join("library.sqlite3")).unwrap();
        choose(&mut database, &a);
        let entry = database.playlist().unwrap().remove(0);
        let mut playback = PlaybackController::new();
        let id = playback
            .begin(database.media(&entry.media_id).unwrap(), Some(entry.id))
            .unwrap();
        let state = AppState {
            settings: Arc::new(Mutex::new(database.load_settings().unwrap())),
            database: Arc::new(Mutex::new(database)),
            playback: Arc::new(Mutex::new(playback)),
            downloads: Arc::new(Mutex::new(DownloadRegistry::default())),
            directory_operation: Arc::new(Mutex::new(())),
            transcription_submission: Arc::new(tokio::sync::Mutex::new(())),
            playlist_snapshot_version: Arc::new(AtomicU64::new(0)),
        };
        assert!(update_directory(&state, Some(temp.path().join("missing"))).is_err());
        assert!(lock(&state.playback).unwrap().matches(id));
        assert!(update_directory(&state, None)
            .unwrap()
            .playback_error
            .is_none());
        assert!(lock(&state.playback).unwrap().matches(id));
        assert!(update_directory(&state, Some(b))
            .unwrap()
            .playback_error
            .is_none());
        assert!(lock(&state.playback).unwrap().session.is_none());
        assert!(!lock(&state.playback).unwrap().is_requested(id));
    }

    #[test]
    fn directory_scan_does_not_follow_external_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("songs");
        std::fs::create_dir(&root).unwrap();
        let external = media_file(&temp.path().join("other"), "outside.mp3");
        #[cfg(windows)]
        {
            let result = crate::services::toolchain::hidden_command("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:DRIP_TEST_LINK_PATH -Target $env:DRIP_TEST_LINK_TARGET -ErrorAction Stop | Out-Null"])
                .env("DRIP_TEST_LINK_PATH", root.join("linked-directory"))
                .env("DRIP_TEST_LINK_TARGET", external.parent().unwrap())
                .output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(external.parent().unwrap(), root.join("linked-directory"))
            .unwrap();
        assert!(scan_directory(&root).unwrap().media.is_empty());
        assert!(!contains_path(&root, &external).unwrap());
    }
}
