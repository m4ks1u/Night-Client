-- 0001: instances and settings.
-- Secrets (tokens) are never stored in this database.

CREATE TABLE instances (
    id                TEXT    PRIMARY KEY NOT NULL,
    name              TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    -- Folder under <data>/instances. Fixed at creation so renames never move files.
    dir_name          TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    game_version      TEXT    NOT NULL,
    loader            TEXT    NOT NULL DEFAULT 'vanilla' CHECK (loader IN ('vanilla', 'fabric', 'neoforge')),
    loader_version    TEXT,
    max_memory_mb     INTEGER CHECK (max_memory_mb IS NULL OR max_memory_mb BETWEEN 512 AND 65536),
    java_path         TEXT,
    jvm_args          TEXT    NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    last_played_at    INTEGER,
    play_time_seconds INTEGER NOT NULL DEFAULT 0 CHECK (play_time_seconds >= 0)
) STRICT;

CREATE INDEX instances_recent ON instances (last_played_at DESC, created_at DESC);

CREATE TABLE settings (
    key        TEXT    PRIMARY KEY NOT NULL,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
