-- 0005: metadata for files in each instance's mods folder.
-- The folder is the source of truth; rows are re-synced when it is listed.

CREATE TABLE instance_mods (
    instance_id    TEXT    NOT NULL REFERENCES instances (id) ON DELETE CASCADE,
    -- Enabled file name (x.jar); a disabled file is x.jar.disabled on disk.
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
    -- Project that pulled this mod in as a required dependency, if any.
    required_by    TEXT,
    installed_at   INTEGER NOT NULL,
    PRIMARY KEY (instance_id, file_name)
) STRICT;

CREATE INDEX instance_mods_project ON instance_mods (instance_id, project_id);
