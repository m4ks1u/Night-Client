//! Night Client local database.
//!
//! One SQLite file (`<data>/night-client.db`, WAL mode) with forward-only,
//! transactional migrations tracked in `PRAGMA user_version`. Before an
//! existing database is migrated, a backup copy is written next to it.
//!
//! The API is synchronous and fast (sub-millisecond queries). Async callers
//! should run it on a blocking thread.

#![forbid(unsafe_code)]

mod accounts;
mod downloads;
mod instances;
pub mod migrations;
mod mods;
mod settings;

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;

pub use accounts::AccountRecord;
pub use downloads::{DownloadRecord, DownloadStatus, HISTORY_LIMIT};
pub use instances::{Instance, InstancePatch, Loader, NewInstance};
pub use mods::{ModRecord, ModSource};
pub use settings::AppSettings;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "the database was created by a newer Night Client (schema v{found}, this build supports v{supported}); update Night Client"
    )]
    NewerSchema { found: u32, supported: u32 },
    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        version: u32,
        name: &'static str,
        source: rusqlite::Error,
    },
    #[error("could not back up the database before migrating: {0}")]
    Backup(String),
    #[error("{0}")]
    Invalid(String),
    #[error("instance not found")]
    NotFound,
    #[error("stored value for `{key}` is corrupt: {message}")]
    Corrupt { key: String, message: String },
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Store {
    /// Open (creating if needed) and migrate the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_migrations(path, migrations::MIGRATIONS)
    }

    /// `open` with an explicit migration list (tests of the upgrade path).
    #[doc(hidden)]
    pub fn open_with_migrations(path: &Path, all: &[migrations::Migration]) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Backup(e.to_string()))?;
        }
        let existed = path.exists();
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        if existed {
            backup_if_migrating(&conn, path, all)?;
        }
        migrations::migrate(&mut conn, all)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        })
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn)?;
        migrations::migrate(&mut conn, migrations::MIGRATIONS)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn schema_version(&self) -> Result<u32> {
        migrations::user_version(&self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means a panic happened mid-call; SQLite itself
        // rolled back any open transaction, so the connection is usable.
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

fn backup_if_migrating(
    conn: &Connection,
    path: &Path,
    all: &[migrations::Migration],
) -> Result<()> {
    let current = migrations::user_version(conn)?;
    let latest = all.last().map(|m| m.version).unwrap_or(0);
    if current == 0 || current >= latest {
        return Ok(());
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".v{current}.bak"));
    let backup = path.with_file_name(name);
    let _ = std::fs::remove_file(&backup);
    conn.execute("VACUUM INTO ?1", [backup.display().to_string()])
        .map_err(|e| StoreError::Backup(e.to_string()))?;
    tracing::info!(from = current, to = latest, backup = %backup.display(), "database backed up before migration");
    Ok(())
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
