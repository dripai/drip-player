use crate::models::playlist::{
    canonical_local_identity, LibraryItem, Playlist, PlaylistEntry, PlaylistItem, PlaylistOrigin,
    PlaylistStateFile,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const PLAYLIST_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub minimize_to_tray: bool,
}

pub struct PersistenceManager;

impl PersistenceManager {
    fn config_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join("config")
    }

    fn playlist_state_path() -> PathBuf {
        Self::config_dir().join("playlist_v2.json")
    }

    fn legacy_playlist_path() -> PathBuf {
        Self::config_dir().join("playlist.json")
    }

    fn legacy_library_path() -> PathBuf {
        Self::config_dir().join("library.json")
    }

    fn legacy_playlist_entries_path() -> PathBuf {
        Self::config_dir().join("playlist_entries.json")
    }

    fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    fn ensure_config_dir() -> Result<PathBuf, String> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir).map_err(|error| {
            format!(
                "Failed to create config directory {}: {error}",
                dir.display()
            )
        })?;
        Ok(dir)
    }

    pub fn load_playlist_state() -> Result<PlaylistStateFile, String> {
        let path = Self::playlist_state_path();
        if let Some(state) = Self::load_optional_json::<PlaylistStateFile>(&path)? {
            Self::validate_playlist_state(&state)?;
            return Ok(state);
        }

        let migrated = Self::migrate_legacy_playlist()?;
        Self::save_playlist_state(&migrated)?;
        Ok(migrated)
    }

    pub fn save_playlist_state(state: &PlaylistStateFile) -> Result<(), String> {
        Self::validate_playlist_state(state)?;
        Self::ensure_config_dir()?;
        let path = Self::playlist_state_path();
        Self::write_playlist_state(&path, state)
    }

    fn write_playlist_state(path: &Path, state: &PlaylistStateFile) -> Result<(), String> {
        let dir = path
            .parent()
            .ok_or_else(|| format!("Playlist path has no parent: {}", path.display()))?;
        fs::create_dir_all(dir)
            .map_err(|error| format!("Failed to create playlist directory: {error}"))?;
        let mut temp = tempfile::NamedTempFile::new_in(&dir)
            .map_err(|error| format!("Failed to create playlist temp file: {error}"))?;
        serde_json::to_writer_pretty(temp.as_file_mut(), state)
            .map_err(|error| format!("Failed to serialize playlist state: {error}"))?;
        temp.as_file_mut()
            .write_all(b"\n")
            .map_err(|error| format!("Failed to finish playlist temp file: {error}"))?;
        temp.as_file_mut()
            .flush()
            .map_err(|error| format!("Failed to flush playlist temp file: {error}"))?;
        temp.as_file()
            .sync_all()
            .map_err(|error| format!("Failed to sync playlist temp file: {error}"))?;
        temp.persist(&path).map_err(|error| {
            format!(
                "Failed to atomically replace playlist state {}: {}",
                path.display(),
                error.error
            )
        })?;
        Ok(())
    }

    fn validate_playlist_state(state: &PlaylistStateFile) -> Result<(), String> {
        if state.schema_version != PLAYLIST_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported playlist schema version: {}",
                state.schema_version
            ));
        }

        let mut ids = HashSet::new();
        let mut canonical_keys = HashSet::new();
        for item in &state.items {
            if item.id.trim().is_empty() || item.canonical_key.trim().is_empty() {
                return Err("Playlist contains an item without a stable identity".to_string());
            }
            if !ids.insert(item.id.clone()) {
                return Err(format!("Duplicate playlist item id: {}", item.id));
            }
            if !canonical_keys.insert(item.canonical_key.clone()) {
                return Err(format!(
                    "Duplicate playlist media identity: {}",
                    item.canonical_key
                ));
            }
        }
        Ok(())
    }

    fn migrate_legacy_playlist() -> Result<PlaylistStateFile, String> {
        let library = Self::load_optional_json::<Vec<LibraryItem>>(&Self::legacy_library_path())?
            .unwrap_or_default();
        let entries =
            Self::load_optional_json::<Vec<PlaylistEntry>>(&Self::legacy_playlist_entries_path())?
                .unwrap_or_default();
        let mut library_by_id = HashMap::new();
        for item in &library {
            Self::collect_library_tracks(item, &mut library_by_id);
        }

        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for entry in entries {
            let Some(library_item) = library_by_id.get(&entry.item_id) else {
                continue;
            };
            let Some(item) = PlaylistItem::from_library_item(library_item, entry.added_at) else {
                continue;
            };
            if seen.insert(item.canonical_key.clone()) {
                items.push(item);
            }
        }

        if items.is_empty() {
            let legacy_playlist =
                Self::load_optional_json::<Playlist>(&Self::legacy_playlist_path())?
                    .unwrap_or_else(Playlist::new);
            for track in &legacy_playlist.tracks {
                let library_item = Playlist::track_to_library_item(track);
                let added_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let Some(item) = PlaylistItem::from_library_item(&library_item, added_at) else {
                    continue;
                };
                if seen.insert(item.canonical_key.clone()) {
                    items.push(item);
                }
            }
        }

        Self::remove_cached_local_duplicates(&mut items);

        Ok(PlaylistStateFile {
            schema_version: PLAYLIST_SCHEMA_VERSION,
            revision: u64::from(!items.is_empty()),
            items,
        })
    }

    fn remove_cached_local_duplicates(items: &mut Vec<PlaylistItem>) {
        let cached_local_keys = items
            .iter()
            .filter_map(|item| match &item.origin {
                PlaylistOrigin::Remote { .. } => item
                    .cached_path
                    .as_ref()
                    .map(|path| canonical_local_identity(path).1),
                PlaylistOrigin::Local { .. } => None,
            })
            .collect::<HashSet<_>>();
        items.retain(|item| {
            !matches!(item.origin, PlaylistOrigin::Local { .. })
                || !cached_local_keys.contains(&item.canonical_key)
        });
    }

    fn collect_library_tracks<'a>(
        item: &'a LibraryItem,
        target: &mut HashMap<String, &'a LibraryItem>,
    ) {
        match item {
            LibraryItem::Track { id, .. } => {
                target.entry(id.clone()).or_insert(item);
            }
            LibraryItem::Folder { children, .. } => {
                for child in children {
                    Self::collect_library_tracks(child, target);
                }
            }
        }
    }

    fn load_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        serde_json::from_str(&content)
            .map(Some)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))
    }

    pub fn save_settings(settings: &AppSettings) {
        if let Ok(dir) = Self::ensure_config_dir() {
            let path = dir.join("settings.json");
            if let Ok(json) = serde_json::to_string_pretty(settings) {
                let _ = fs::write(path, json);
            }
        }
    }

    pub fn load_settings() -> AppSettings {
        Self::load_optional_json(&Self::settings_path())
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::playlist::MediaType;

    fn playlist_item(id: &str, canonical_key: &str) -> PlaylistItem {
        PlaylistItem {
            id: id.to_string(),
            canonical_key: canonical_key.to_string(),
            title: id.to_string(),
            media_type: MediaType::Audio,
            origin: PlaylistOrigin::Local {
                path: PathBuf::from(format!("C:/media/{id}.mp3")),
            },
            cached_path: None,
            added_at: 1,
        }
    }

    #[test]
    fn playlist_state_rejects_duplicate_media_identity() {
        let state = PlaylistStateFile {
            schema_version: PLAYLIST_SCHEMA_VERSION,
            revision: 1,
            items: vec![
                playlist_item("one", "local:c:/media/song.mp3"),
                playlist_item("two", "local:c:/media/song.mp3"),
            ],
        };

        assert!(PersistenceManager::validate_playlist_state(&state).is_err());
    }

    #[test]
    fn playlist_state_atomically_replaces_existing_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("playlist_v2.json");
        let mut state = PlaylistStateFile {
            schema_version: PLAYLIST_SCHEMA_VERSION,
            revision: 1,
            items: vec![playlist_item("one", "local:c:/media/one.mp3")],
        };
        PersistenceManager::write_playlist_state(&path, &state).unwrap();

        state.revision = 2;
        state.items = vec![playlist_item("two", "local:c:/media/two.mp3")];
        PersistenceManager::write_playlist_state(&path, &state).unwrap();

        let loaded: PlaylistStateFile =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(loaded.revision, 2);
        assert_eq!(loaded.items[0].id, "two");
    }

    #[test]
    fn migration_drops_local_row_that_is_a_remote_cache_file() {
        let cached_path = PathBuf::from("C:/cache/video.mp4");
        let (_, local_key) = canonical_local_identity(&cached_path);
        let mut items = vec![
            PlaylistItem {
                id: "local".to_string(),
                canonical_key: local_key,
                title: "video".to_string(),
                media_type: MediaType::Video,
                origin: PlaylistOrigin::Local {
                    path: cached_path.clone(),
                },
                cached_path: None,
                added_at: 1,
            },
            PlaylistItem {
                id: "remote".to_string(),
                canonical_key: "remote:youtube:video".to_string(),
                title: "video".to_string(),
                media_type: MediaType::Video,
                origin: PlaylistOrigin::Remote {
                    url: "https://youtu.be/video".to_string(),
                    provider: "youtube".to_string(),
                    external_id: "video".to_string(),
                },
                cached_path: Some(cached_path),
                added_at: 2,
            },
        ];

        PersistenceManager::remove_cached_local_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "remote");
    }
}
