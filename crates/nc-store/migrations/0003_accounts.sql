-- 0003: non-sensitive Microsoft account metadata.
-- Refresh tokens live in the DPAPI vault, never here.

CREATE TABLE accounts (
    uuid         TEXT    PRIMARY KEY NOT NULL CHECK (length(uuid) = 32),
    name         TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 32),
    added_at     INTEGER NOT NULL,
    last_used_at INTEGER,
    is_active    INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1))
) STRICT;

-- At most one active account.
CREATE UNIQUE INDEX accounts_single_active ON accounts (is_active) WHERE is_active = 1;
