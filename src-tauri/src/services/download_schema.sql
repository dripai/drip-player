CREATE TABLE IF NOT EXISTS download_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
