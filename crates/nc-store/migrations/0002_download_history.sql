-- 0002: history of finished download jobs (installs and repairs).

CREATE TABLE download_history (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    -- No CHECK on kind: SQLite cannot alter CHECK constraints, and new job
    -- kinds (mods, loaders) will be added later. Validated in Rust.
    kind             TEXT    NOT NULL,
    target           TEXT    NOT NULL,
    status           TEXT    NOT NULL CHECK (status IN ('completed', 'failed', 'cancelled')),
    repair           INTEGER NOT NULL DEFAULT 0 CHECK (repair IN (0, 1)),
    queued_at        INTEGER NOT NULL,
    started_at       INTEGER NOT NULL,
    finished_at      INTEGER NOT NULL,
    files_downloaded INTEGER NOT NULL DEFAULT 0,
    files_verified   INTEGER NOT NULL DEFAULT 0,
    bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    -- JSON array of data-relative paths re-downloaded during a repair (≤ 200).
    repaired         TEXT    NOT NULL DEFAULT '[]',
    error            TEXT
) STRICT;

CREATE INDEX download_history_recent ON download_history (finished_at DESC);
