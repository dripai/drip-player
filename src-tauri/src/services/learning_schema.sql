CREATE TABLE IF NOT EXISTS learning_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    data TEXT NOT NULL CHECK (json_valid(data))
) STRICT;
CREATE TABLE IF NOT EXISTS transcripts (
    id TEXT PRIMARY KEY,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE RESTRICT,
    label TEXT NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data))
) STRICT;
CREATE INDEX IF NOT EXISTS transcripts_media ON transcripts(media_id);
CREATE TABLE IF NOT EXISTS learning_cue_notes (
    transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE RESTRICT,
    cue_id INTEGER NOT NULL CHECK (cue_id >= 0),
    translation TEXT,
    translation_language TEXT,
    favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    PRIMARY KEY (transcript_id, cue_id)
) STRICT;
CREATE TABLE IF NOT EXISTS learning_recordings (
    id TEXT PRIMARY KEY,
    transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE RESTRICT,
    cue_id INTEGER NOT NULL CHECK (cue_id >= 0),
    path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    evaluation TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS learning_transcription_jobs (
    id TEXT PRIMARY KEY,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE RESTRICT,
    settings TEXT NOT NULL CHECK (json_valid(settings)),
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'failed')),
    error TEXT,
    transcript_id TEXT REFERENCES transcripts(id)
) STRICT;
