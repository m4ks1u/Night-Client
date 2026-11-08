use rusqlite::Connection;

use crate::{Result, StoreError};

/// A forward-only schema migration. Versions must be 1, 2, 3, … in order.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "download_history",
        sql: include_str!("../migrations/0002_download_history.sql"),
    },
    Migration {
        version: 3,
        name: "accounts",
        sql: include_str!("../migrations/0003_accounts.sql"),
    },
    Migration {
        version: 4,
        name: "instance_game_dir",
        sql: include_str!("../migrations/0004_instance_game_dir.sql"),
    },
    Migration {
        version: 5,
        name: "instance_mods",
        sql: include_str!("../migrations/0005_instance_mods.sql"),
    },
];

pub fn user_version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Apply every migration newer than the database's `user_version`, each in
/// its own transaction together with the version bump, so a failure leaves
/// the database at the last good version. Returns the final version.
pub fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<u32> {
    for (i, m) in migrations.iter().enumerate() {
        assert_eq!(
            m.version as usize,
            i + 1,
            "migration versions must be contiguous from 1"
        );
    }
    let latest = migrations.last().map(|m| m.version).unwrap_or(0);
    let current = user_version(conn)?;
    if current > latest {
        return Err(StoreError::NewerSchema {
            found: current,
            supported: latest,
        });
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let wrap = |source| StoreError::Migration {
            version: m.version,
            name: m.name,
            source,
        };
        let tx = conn.transaction().map_err(wrap)?;
        tx.execute_batch(m.sql).map_err(wrap)?;
        tx.pragma_update(None, "user_version", m.version)
            .map_err(wrap)?;
        tx.commit().map_err(wrap)?;
        tracing::info!(version = m.version, name = m.name, "applied migration");
    }
    user_version(conn)
}
