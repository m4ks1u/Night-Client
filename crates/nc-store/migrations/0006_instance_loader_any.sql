-- 0006: drop the CHECK on instances.loader so new loaders (Quilt first) need
-- no table rebuild. The application validates loader names on every read
-- and write (nc_store::Loader). SQLite cannot alter a CHECK, so the table
-- is rebuilt. Dropping `instances` with foreign keys on would cascade to
-- `instance_mods`, so mod rows are saved first and restored afterwards.
-- Runs in the migration's transaction; a failure changes nothing.

CREATE TEMP TABLE saved_mods AS SELECT * FROM instance_mods;
DROP TABLE instance_mods;

CREATE TABLE instances_new (
    id                TEXT    PRIMARY KEY NOT NULL,
    name              TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    dir_name          TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    game_version      TEXT    NOT NULL,
    loader            TEXT    NOT NULL DEFAULT 'vanilla' CHECK (length(loader) BETWEEN 1 AND 32),
    loader_version    TEXT,
    max_memory_mb     INTEGER CHECK (max_memory_mb IS NULL OR max_memory_mb BETWEEN 512 AND 65536),
    java_path         TEXT,
    jvm_args          TEXT    NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    last_played_at    INTEGER,
    play_time_seconds INTEGER NOT NULL DEFAULT 0 CHECK (play_time_seconds >= 0),
    game_dir          TEXT    CHECK (game_dir IS NULL OR length(game_dir) BETWEEN 3 AND 1024)
) STRICT;

INSERT INTO instances_new (id, name, dir_name, game_version, loader, loader_version, max_memory_mb,
                           java_path, jvm_args, created_at, last_played_at, play_time_seconds, game_dir)
SELECT id, name, dir_name, game_version, loader, loader_version, max_memory_mb,
       java_path, jvm_args, created_at, last_played_at, play_time_seconds, game_dir
FROM instances;

DROP TABLE instances;
ALTER TABLE instances_new RENAME TO instances;
CREATE INDEX instances_recent ON instances (last_played_at DESC, created_at DESC);

CREATE TABLE instance_mods (
    instance_id    TEXT    NOT NULL REFERENCES instances (id) ON DELETE CASCADE,
    file_name      TEXT    NOT NULL CHECK (length(file_name) BETWEEN 5 AND 200),
    source         TEXT    NOT NULL CHECK (source IN ('modrinth', 'local')),
    project_id     TEXT,
    version_id     TEXT,
    title          TEXT    NOT NULL,
    version_number TEXT,
    icon_url       TEXT,
    sha1           TEXT    NOT NULL,
    sha512         TEXT    NOT NULL,
    size           INTEGER NOT NULL,
    enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    required_by    TEXT,
    installed_at   INTEGER NOT NULL,
    PRIMARY KEY (instance_id, file_name)
) STRICT;
CREATE INDEX instance_mods_project ON instance_mods (instance_id, project_id);

INSERT INTO instance_mods SELECT * FROM saved_mods;
DROP TABLE saved_mods;
