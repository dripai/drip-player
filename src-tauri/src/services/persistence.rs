use crate::models::playlist::{MediaType, PlaylistItem, PlaylistOrigin};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Serialize)]
pub struct AppSettings {
    pub revision: u64,
    pub theme: String,
    pub language: String,
    pub play_mode: String,
    pub minimize_to_tray: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettingsPatch {
    pub theme: Option<String>,
    pub language: Option<String>,
    pub play_mode: Option<String>,
    pub minimize_to_tray: Option<bool>,
}

pub struct StoredPlaylist {
    pub revision: u64,
    pub items: Vec<PlaylistItem>,
}

pub struct PersistenceManager {
    connection: Connection,
}

fn database_error(error: rusqlite::Error) -> String {
    format!("SQLite: {error}")
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Media path cannot be represented as UTF-8".to_string())
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
        let path = directory.join("drip-player.sqlite3");
        let mut connection = Connection::open(&path)
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
            _ => return Err(format!("Unsupported SQLite schema version: {version}")),
        }
        transaction.commit().map_err(database_error)?;
        Ok(Self { connection })
    }

    pub fn load_playlist(&self) -> Result<StoredPlaylist, String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let revision = transaction
            .query_row(
                "SELECT revision FROM playlist_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let items = {
            let mut statement = transaction
                .prepare(
                    "SELECT p.id, m.canonical_key, m.title, m.media_type, m.origin_kind,
                            m.local_path, m.remote_url, m.provider, m.external_id, m.cached_path, p.added_at, m.id
                     FROM playlist_entries p JOIN media m ON m.id = p.media_id ORDER BY p.position",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    let media_type: String = row.get(3)?;
                    let origin_kind: String = row.get(4)?;
                    let media_type = match media_type.as_str() {
                        "Audio" => MediaType::Audio,
                        "Video" => MediaType::Video,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    let origin = match origin_kind.as_str() {
                        "local" => PlaylistOrigin::Local {
                            path: PathBuf::from(row.get::<_, String>(5)?),
                        },
                        "remote" => PlaylistOrigin::Remote {
                            url: row.get(6)?,
                            provider: row.get(7)?,
                            external_id: row.get(8)?,
                        },
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(PlaylistItem {
                        id: row.get(0)?,
                        media_id: row.get(11)?,
                        canonical_key: row.get(1)?,
                        title: row.get(2)?,
                        media_type,
                        origin,
                        cached_path: row.get::<_, Option<String>>(9)?.map(PathBuf::from),
                        added_at: row.get(10)?,
                    })
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        transaction.commit().map_err(database_error)?;
        Ok(StoredPlaylist { revision, items })
    }

    pub fn replace_playlist(
        &mut self,
        expected_revision: u64,
        mut items: Vec<PlaylistItem>,
    ) -> Result<StoredPlaylist, String> {
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for item in &items {
            if item.id.trim().is_empty()
                || item.media_id.trim().is_empty()
                || item.canonical_key.trim().is_empty()
            {
                return Err("Playlist item has no stable identity".to_string());
            }
            if !ids.insert(&item.id) || !keys.insert(&item.canonical_key) {
                return Err("Playlist contains duplicate media".to_string());
            }
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| "Playlist revision overflow".to_string())?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let updated = transaction
            .execute(
                "UPDATE playlist_state SET revision = ?1 WHERE id = 1 AND revision = ?2",
                params![revision, expected_revision],
            )
            .map_err(database_error)?;
        if updated != 1 {
            return Err("Playlist changed in another process; restart to reload it".to_string());
        }
        transaction
            .execute("DELETE FROM playlist_entries", [])
            .map_err(database_error)?;
        for (position, item) in items.iter_mut().enumerate() {
            let (origin_kind, local_path, remote_url, provider, external_id) = match &item.origin {
                PlaylistOrigin::Local { path } => {
                    ("local", Some(path_text(path)?), None, None, None)
                }
                PlaylistOrigin::Remote {
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
            let media_type = match item.media_type {
                MediaType::Audio => "Audio",
                MediaType::Video => "Video",
            };
            transaction
                .execute(
                    "INSERT INTO media (id, canonical_key, title, media_type, origin_kind,
                     local_path, remote_url, provider, external_id, cached_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(canonical_key) DO UPDATE SET
                     title = excluded.title, media_type = excluded.media_type,
                     origin_kind = excluded.origin_kind, local_path = excluded.local_path,
                     remote_url = excluded.remote_url, provider = excluded.provider,
                     external_id = excluded.external_id, cached_path = excluded.cached_path",
                    params![
                        item.media_id,
                        item.canonical_key,
                        item.title,
                        media_type,
                        origin_kind,
                        local_path,
                        remote_url,
                        provider,
                        external_id,
                        item.cached_path.as_deref().map(path_text).transpose()?
                    ],
                )
                .map_err(database_error)?;
            // Re-adding media restores its existing identity for learning references.
            item.media_id = transaction
                .query_row(
                    "SELECT id FROM media WHERE canonical_key = ?1",
                    [&item.canonical_key],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            transaction.execute(
                "INSERT INTO playlist_entries (id, media_id, position, added_at) VALUES (?1, ?2, ?3, ?4)",
                params![item.id, item.media_id, position, item.added_at],
            ).map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;
        Ok(StoredPlaylist { revision, items })
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        self.connection.query_row(
            "SELECT revision, theme, language, play_mode, minimize_to_tray FROM app_settings WHERE id = 1",
            [],
            |row| Ok(AppSettings {
                revision: row.get(0)?,
                theme: row.get(1)?,
                language: row.get(2)?,
                play_mode: row.get(3)?,
                minimize_to_tray: row.get(4)?,
            }),
        ).map_err(database_error)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let previous_revision = settings
            .revision
            .checked_sub(1)
            .ok_or_else(|| "Invalid settings revision".to_string())?;
        let updated = self
            .connection
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
}
