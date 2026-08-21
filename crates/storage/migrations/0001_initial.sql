-- Initial schema for agpeer.
-- Timestamps are stored as RFC3339 text (UTC). Ids are stored as canonical
-- text representations of UUIDs.

CREATE TABLE IF NOT EXISTS transfers (
    id                  TEXT PRIMARY KEY,
    backend             TEXT NOT NULL,
    source              TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    state               TEXT NOT NULL,
    progress            REAL NOT NULL DEFAULT 0,
    bytes_total         INTEGER,
    bytes_completed     INTEGER NOT NULL DEFAULT 0,
    download_rate       INTEGER,
    upload_rate         INTEGER,
    eta                 INTEGER,
    destination         TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    started_at          TEXT,
    completed_at        TEXT,
    error               TEXT,
    postprocess_state   TEXT NOT NULL DEFAULT 'none',
    metadata            TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_transfers_state ON transfers (state);
CREATE INDEX IF NOT EXISTS idx_transfers_backend ON transfers (backend);

CREATE TABLE IF NOT EXISTS transfer_files (
    transfer_id      TEXT NOT NULL,
    file_index       TEXT NOT NULL,
    path             TEXT NOT NULL,
    size             INTEGER NOT NULL DEFAULT 0,
    selected         INTEGER NOT NULL DEFAULT 1,
    bytes_completed  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (transfer_id, file_index)
);

CREATE TABLE IF NOT EXISTS searches (
    id            TEXT PRIMARY KEY,
    backend       TEXT NOT NULL,
    query         TEXT NOT NULL,
    state         TEXT NOT NULL,
    result_count  INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    expires_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_searches_expires ON searches (expires_at);

CREATE TABLE IF NOT EXISTS search_results (
    result_id         TEXT PRIMARY KEY,
    search_id         TEXT NOT NULL,
    username          TEXT NOT NULL,
    path              TEXT NOT NULL,
    filename          TEXT NOT NULL,
    size              INTEGER,
    extension         TEXT,
    bitrate           INTEGER,
    duration          INTEGER,
    attributes        TEXT NOT NULL DEFAULT '{}',
    queue_length      INTEGER,
    free_upload_slots INTEGER,
    upload_speed      INTEGER,
    backend_metadata  TEXT NOT NULL DEFAULT '{}',
    expires_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_results_search ON search_results (search_id);
CREATE INDEX IF NOT EXISTS idx_results_expires ON search_results (expires_at);

CREATE TABLE IF NOT EXISTS postprocess_jobs (
    id           TEXT PRIMARY KEY,
    transfer_id  TEXT NOT NULL,
    target       TEXT NOT NULL,
    state        TEXT NOT NULL,
    steps        TEXT NOT NULL DEFAULT '[]',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    error        TEXT
);

CREATE INDEX IF NOT EXISTS idx_jobs_transfer ON postprocess_jobs (transfer_id);

CREATE TABLE IF NOT EXISTS postprocess_steps (
    job_id        TEXT NOT NULL,
    step_index    INTEGER NOT NULL,
    kind          TEXT NOT NULL,
    state         TEXT NOT NULL,
    started_at    TEXT,
    completed_at  TEXT,
    error         TEXT,
    PRIMARY KEY (job_id, step_index)
);

CREATE TABLE IF NOT EXISTS settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts       TEXT NOT NULL,
    kind     TEXT NOT NULL,
    payload  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_ts ON events (ts);

CREATE TABLE IF NOT EXISTS backend_state (
    backend     TEXT PRIMARY KEY,
    state       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
