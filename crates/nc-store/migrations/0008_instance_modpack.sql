-- 0008: which Modrinth modpack an instance was made from, and the files the
-- pack put there (with their SHA-1 at install time). A modpack update uses
-- the hashes to tell files the user changed (kept) from files it may
-- replace or remove. Both tables go away with the instance.

CREATE TABLE instance_modpack (
    instance_id    TEXT    PRIMARY KEY REFERENCES instances (id) ON DELETE CASCADE,
    -- NULL for a pack imported from a file that Modrinth doesn't know.
    project_id     TEXT,
    version_id     TEXT,
    name           TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    version_number TEXT    NOT NULL CHECK (length(version_number) BETWEEN 1 AND 200),
    icon_url       TEXT,
    installed_at   INTEGER NOT NULL
) STRICT;

CREATE TABLE instance_modpack_files (
    instance_id TEXT    NOT NULL REFERENCES instance_modpack (instance_id) ON DELETE CASCADE,
    -- Relative path inside the game folder, '/'-separated, as the pack names it.
    path        TEXT    NOT NULL CHECK (length(path) BETWEEN 1 AND 512),
    sha1        TEXT    NOT NULL CHECK (length(sha1) = 40),
    -- 1: copied from overrides/ (configs, options); 0: a downloaded file.
    is_override INTEGER NOT NULL CHECK (is_override IN (0, 1)),
    PRIMARY KEY (instance_id, path)
) STRICT;
