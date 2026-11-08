-- 0007: instance_mods also tracks resource packs and shader packs. Each row
-- gets a `kind`, and the key includes it: the same file name may exist in
-- mods/, resourcepacks/ and shaderpacks/. Existing rows are mods. SQLite
-- cannot change a primary key, so the table is rebuilt (instance_mods has
-- no dependants, so nothing cascades). Runs in the migration's transaction.

CREATE TABLE instance_content (
    instance_id    TEXT    NOT NULL REFERENCES instances (id) ON DELETE CASCADE,
    kind           TEXT    NOT NULL DEFAULT 'mod' CHECK (kind IN ('mod', 'resourcepack', 'shader')),
    -- Enabled file name (x.jar / x.zip); a disabled file is <name>.disabled on disk.
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
    PRIMARY KEY (instance_id, kind, file_name)
) STRICT;

INSERT INTO instance_content (instance_id, kind, file_name, source, project_id, version_id, title,
                              version_number, icon_url, sha1, sha512, size, enabled, required_by,
                              installed_at)
SELECT instance_id, 'mod', file_name, source, project_id, version_id, title,
       version_number, icon_url, sha1, sha512, size, enabled, required_by, installed_at
FROM instance_mods;

DROP TABLE instance_mods;
ALTER TABLE instance_content RENAME TO instance_mods;
CREATE INDEX instance_mods_project ON instance_mods (instance_id, kind, project_id);
