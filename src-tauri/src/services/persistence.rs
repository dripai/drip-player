use crate::models::media::{AssetKind, AssetSource, Media, MediaAsset, MediaOrigin, MediaType};
use crate::models::playlist::PlaylistEntry;
use crate::models::settings::AppSettings;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA_VERSION: i64 = 2;

pub struct PersistenceManager {
    pub(super) connection: Connection,
    pub(super) playlist_revision: u64,
    _instance_lock: Option<fslock::LockFile>,
}

pub(super) fn acquire_instance_lock(path: &Path) -> Result<fslock::LockFile, String> {
    let mut file =
        fslock::LockFile::open(path).map_err(|error| format!("无法打开应用锁：{error}"))?;
    if !file
        .try_lock()
        .map_err(|error| format!("无法锁定应用数据：{error}"))?
    {
        return Err("此配置目录已有播放器在运行，请先退出另一个实例".into());
    }
    Ok(file)
}

pub(super) fn database_error(error: rusqlite::Error) -> String {
    format!("SQLite: {error}")
}
pub(super) fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Media path is not UTF-8".into())
}

pub(super) fn read_media(connection: &Connection, id: &str) -> Result<Media, String> {
    let mut media = connection.query_row(
        "SELECT id, canonical_key, title, media_type, origin_kind, local_path, remote_url, provider, external_id FROM media WHERE id = ?1",
        [id], |row| {
            let origin: String = row.get(4)?;
            let kind: String = row.get(3)?;
            Ok(Media { id: row.get(0)?, canonical_key: row.get(1)?, title: row.get(2)?,
                media_type: if kind == "Video" { MediaType::Video } else { MediaType::Audio },
                origin: if origin == "local" { MediaOrigin::Local { path: PathBuf::from(row.get::<_, String>(5)?) } }
                    else { MediaOrigin::Remote { url: row.get(6)?, provider: row.get(7)?, external_id: row.get(8)? } },
                assets: Vec::new() })
        }).map_err(database_error)?;
    let mut query = connection.prepare("SELECT id, kind, path, language, source FROM media_assets WHERE media_id = ?1 ORDER BY kind, language, path").map_err(database_error)?;
    media.assets = query
        .query_map([id], |row| {
            let kind: String = row.get(1)?;
            let source: String = row.get(4)?;
            Ok(MediaAsset {
                id: row.get(0)?,
                media_id: id.into(),
                kind: if kind == "playback" {
                    AssetKind::Playback
                } else {
                    AssetKind::Subtitle
                },
                path: PathBuf::from(row.get::<_, String>(2)?),
                language: row.get(3)?,
                source: if source == "local" {
                    AssetSource::Local
                } else {
                    AssetSource::Download
                },
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(media)
}

pub(super) fn put_assets(
    tx: &Transaction<'_>,
    media_id: &str,
    assets: &[MediaAsset],
) -> Result<(), String> {
    for asset in assets {
        let kind = match asset.kind {
            AssetKind::Playback => "playback",
            AssetKind::Subtitle => "subtitle",
        };
        let source = match asset.source {
            AssetSource::Local => "local",
            AssetSource::Download => "download",
        };
        tx.execute("INSERT INTO media_assets (id, media_id, kind, path, language, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(media_id, path) DO UPDATE SET language = excluded.language, kind = excluded.kind, source = excluded.source",
            params![asset.id, media_id, kind, path_text(&asset.path)?, asset.language, source]).map_err(database_error)?;
    }
    Ok(())
}

pub(super) fn put_media(tx: &Transaction<'_>, media: &Media) -> Result<String, String> {
    let (origin, local, url, provider, external) = match &media.origin {
        MediaOrigin::Local { path } => ("local", Some(path_text(path)?), None, None, None),
        MediaOrigin::Remote {
            url,
            provider,
            external_id,
        } => (
            "remote",
            None,
            Some(url.as_str()),
            Some(provider.as_str()),
            Some(external_id.as_str()),
        ),
    };
    let kind = match media.media_type {
        MediaType::Audio => "Audio",
        MediaType::Video => "Video",
    };
    tx.execute("INSERT INTO media (id, canonical_key, title, media_type, origin_kind, local_path, remote_url, provider, external_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(canonical_key) DO NOTHING",
        params![media.id, media.canonical_key, media.title, kind, origin, local, url, provider, external]).map_err(database_error)?;
    let id: String = tx
        .query_row(
            "SELECT id FROM media WHERE canonical_key = ?1",
            [&media.canonical_key],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    put_assets(tx, &id, &media.assets)?;
    Ok(id)
}

pub(super) fn check_revision(connection: &Connection, expected: u64) -> Result<(), String> {
    let revision: u64 = connection
        .query_row(
            "SELECT revision FROM playlist_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if revision != expected {
        return Err("Playlist changed in another process; restart to reload it".into());
    }
    Ok(())
}

pub(super) fn advance_revision(tx: &Transaction<'_>, expected: u64) -> Result<u64, String> {
    let next = expected
        .checked_add(1)
        .ok_or("Playlist revision overflow")?;
    let count = tx
        .execute(
            "UPDATE playlist_state SET revision = ?1 WHERE id = 1 AND revision = ?2",
            params![next, expected],
        )
        .map_err(database_error)?;
    if count != 1 {
        return Err("Playlist changed in another process; restart to reload it".into());
    }
    Ok(next)
}

impl PersistenceManager {
    pub fn open() -> Result<Self, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("Failed to locate executable: {error}"))?;
        let directory = executable
            .parent()
            .ok_or_else(|| "Executable has no parent directory".to_string())?
            .join("config");
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Failed to create {}: {error}", directory.display()))?;
        let path = directory.join("shadow-player.sqlite3");
        // Recovery must never interrupt tasks owned by another live application instance.
        let instance_lock = acquire_instance_lock(&directory.join("shadow-player.lock"))?;
        let mut database = Self::open_path(&path)?;
        database._instance_lock = Some(instance_lock);
        Ok(database)
    }

    pub(super) fn open_path(path: &Path) -> Result<Self, String> {
        let mut connection = Connection::open(path)
            .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(database_error)?;
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(database_error)?;
        if journal_mode != "wal" {
            return Err(format!("SQLite WAL mode unavailable: {journal_mode}"));
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(database_error)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(database_error)?;
        match version {
            0 => {
                let tables: i64 = transaction
                    .query_row(
                        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(database_error)?;
                if tables != 0 {
                    return Err("SQLite database contains an unrecognized schema".to_string());
                }
                transaction
                    .execute_batch(include_str!("schema.sql"))
                    .map_err(database_error)?;
            }
            SCHEMA_VERSION => {}
            _ => return Err(format!("Unsupported SQLite schema version {version} in {} (expected {SCHEMA_VERSION}); back up and recreate this development database. No migration is provided.", path.display())),
        }
        transaction
            .execute_batch(include_str!("learning_schema.sql"))
            .map_err(database_error)?;
        let preferences = crate::models::learning::LearningSettings::default();
        transaction.execute("INSERT INTO learning_settings (id, revision, data) VALUES (1, 0, ?1) ON CONFLICT(id) DO NOTHING",
            [serde_json::to_string(&preferences).map_err(|e| e.to_string())?]).map_err(database_error)?;
        crate::services::directory_library::initialize(&transaction)?;
        transaction
            .execute_batch(include_str!("download_schema.sql"))
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        let playlist_revision = connection
            .query_row(
                "SELECT revision FROM playlist_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        Ok(Self {
            connection,
            playlist_revision,
            _instance_lock: None,
        })
    }

    pub fn media(&self, id: &str) -> Result<Media, String> {
        read_media(&self.connection, id)
    }

    pub fn set_media_type(&self, id: &str, kind: &MediaType) -> Result<(), String> {
        let kind = match kind {
            MediaType::Audio => "Audio",
            MediaType::Video => "Video",
        };
        let updated = self
            .connection
            .execute(
                "UPDATE media SET media_type = ?1 WHERE id = ?2",
                params![kind, id],
            )
            .map_err(database_error)?;
        if updated != 1 {
            return Err("Media no longer exists".into());
        }
        Ok(())
    }

    pub fn register_media(&mut self, media: &Media) -> Result<Media, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let id = put_media(&tx, media)?;
        let result = read_media(&tx, &id)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }

    pub fn attach_assets(
        &mut self,
        media_id: &str,
        assets: &[MediaAsset],
    ) -> Result<Media, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        put_assets(&tx, media_id, assets)?;
        let result = read_media(&tx, media_id)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }

    pub fn playlist(&self) -> Result<Vec<PlaylistEntry>, String> {
        check_revision(&self.connection, self.playlist_revision)?;
        let mut query = self
            .connection
            .prepare("SELECT id, media_id, added_at FROM playlist_entries ORDER BY position")
            .map_err(database_error)?;
        let entries = query
            .query_map([], |row| {
                Ok(PlaylistEntry {
                    id: row.get(0)?,
                    media_id: row.get(1)?,
                    added_at: row.get(2)?,
                })
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        Ok(entries)
    }

    pub fn entry_media(&self, entry_id: &str) -> Result<Media, String> {
        check_revision(&self.connection, self.playlist_revision)?;
        let id: String = self
            .connection
            .query_row(
                "SELECT media_id FROM playlist_entries WHERE id = ?1",
                [entry_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        self.media(&id)
    }

    pub fn add_to_playlist(&mut self, media: &[Media]) -> Result<Vec<(String, bool)>, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        let mut results = Vec::new();
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();
        for item in media {
            let media_id = put_media(&tx, item)?;
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM playlist_entries WHERE media_id = ?1",
                    [&media_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if let Some(id) = existing {
                results.push((id, false));
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO playlist_entries (id, media_id, position, added_at) SELECT ?1, ?2, COALESCE(MAX(position), -1) + 1, ?3 FROM playlist_entries",
                params![id, media_id, added_at]).map_err(database_error)?;
            results.push((id, true));
        }
        let next = if results.iter().any(|(_, added)| *added) {
            advance_revision(&tx, self.playlist_revision)?
        } else {
            self.playlist_revision
        };
        tx.commit().map_err(database_error)?;
        self.playlist_revision = next;
        Ok(results)
    }

    pub fn remove_entry(&mut self, id: Option<&str>) -> Result<(), String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_revision(&tx, self.playlist_revision)?;
        let removed = if let Some(id) = id {
            tx.execute("DELETE FROM playlist_entries WHERE id = ?1", [id])
        } else {
            tx.execute("DELETE FROM playlist_entries", [])
        }
        .map_err(database_error)?;
        let next = if removed > 0 {
            advance_revision(&tx, self.playlist_revision)?
        } else {
            self.playlist_revision
        };
        tx.commit().map_err(database_error)?;
        self.playlist_revision = next;
        Ok(())
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        self.connection
            .query_row(
                "SELECT s.revision, s.theme, s.language, s.play_mode, s.minimize_to_tray, d.path
             FROM app_settings s JOIN download_directory d ON d.id = s.id WHERE s.id = 1",
                [],
                |row| {
                    Ok(AppSettings {
                        revision: row.get(0)?,
                        theme: row.get(1)?,
                        language: row.get(2)?,
                        play_mode: row.get(3)?,
                        minimize_to_tray: row.get(4)?,
                        download_directory: row.get(5)?,
                    })
                },
            )
            .map_err(database_error)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        write_settings(&self.connection, settings)
    }
}

pub(super) fn write_settings(
    connection: &Connection,
    settings: &AppSettings,
) -> Result<(), String> {
    let previous_revision = settings
        .revision
        .checked_sub(1)
        .ok_or_else(|| "Invalid settings revision".to_string())?;
    let updated = connection
        .execute(
            "UPDATE app_settings SET revision = ?1, theme = ?2, language = ?3,
                play_mode = ?4, minimize_to_tray = ?5 WHERE id = 1 AND revision = ?6",
            params![
                settings.revision,
                settings.theme,
                settings.language,
                settings.play_mode,
                settings.minimize_to_tray,
                previous_revision
            ],
        )
        .map_err(database_error)?;
    if updated != 1 {
        return Err("Settings changed in another process; restart to reload them".to_string());
    }
    Ok(())
}
