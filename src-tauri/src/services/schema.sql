CREATE TABLE media (
    id TEXT PRIMARY KEY NOT NULL,
    canonical_key TEXT NOT NULL UNIQUE CHECK (length(trim(canonical_key)) > 0),
    title TEXT NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN ('Audio', 'Video')),
    origin_kind TEXT NOT NULL CHECK (origin_kind IN ('local', 'remote')),
    local_path TEXT,
    remote_url TEXT,
    provider TEXT,
    external_id TEXT,
    CHECK (
        (origin_kind = 'local' AND local_path IS NOT NULL
            AND remote_url IS NULL AND provider IS NULL AND external_id IS NULL)
        OR
        (origin_kind = 'remote' AND local_path IS NULL
            AND remote_url IS NOT NULL AND provider IS NOT NULL AND external_id IS NOT NULL)
    )
) STRICT;

CREATE TABLE media_assets (
    id TEXT PRIMARY KEY NOT NULL,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('playback', 'subtitle')),
    path TEXT NOT NULL CHECK (length(trim(path)) > 0),
    language TEXT,
    source TEXT NOT NULL CHECK (source IN ('local', 'download')),
    UNIQUE (media_id, path),
    CHECK ((kind = 'playback' AND language IS NULL) OR kind = 'subtitle')
) STRICT;
CREATE UNIQUE INDEX one_playback_asset_per_media ON media_assets(media_id) WHERE kind = 'playback';

-- Membership can be removed without deleting the underlying media identity.
CREATE TABLE playlist_entries (
    id TEXT PRIMARY KEY NOT NULL,
    media_id TEXT NOT NULL UNIQUE REFERENCES media(id) ON DELETE RESTRICT,
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    added_at INTEGER NOT NULL CHECK (added_at >= 0)
) STRICT;

CREATE TABLE playlist_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;
INSERT INTO playlist_state (id, revision) VALUES (1, 0);

CREATE TABLE app_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    theme TEXT NOT NULL CHECK (theme IN ('auto', 'light', 'dark')),
    language TEXT NOT NULL CHECK (language IN ('zh', 'en')),
    play_mode TEXT NOT NULL CHECK (play_mode IN ('sequential', 'random', 'repeat_one', 'repeat_all')),
    minimize_to_tray INTEGER NOT NULL CHECK (minimize_to_tray IN (0, 1))
) STRICT;
INSERT INTO app_settings (id, revision, theme, language, play_mode, minimize_to_tray)
VALUES (1, 0, 'auto', 'zh', 'sequential', 0);

PRAGMA user_version = 2;
