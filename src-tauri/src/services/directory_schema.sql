CREATE TABLE IF NOT EXISTS download_directory (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    path TEXT NOT NULL CHECK (length(trim(path)) > 0),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;
