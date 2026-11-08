use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::{Result, Store, StoreError, now_unix};

/// Non-sensitive account metadata. Tokens are stored in the OS vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub uuid: String,
    pub name: String,
    pub added_at: i64,
    pub last_used_at: Option<i64>,
    pub active: bool,
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<AccountRecord> {
    Ok(AccountRecord {
        uuid: r.get(0)?,
        name: r.get(1)?,
        added_at: r.get(2)?,
        last_used_at: r.get(3)?,
        active: r.get::<_, i64>(4)? == 1,
    })
}

fn validate(uuid: &str, name: &str) -> Result<()> {
    if uuid.len() != 32 || !uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StoreError::Invalid("invalid profile id".into()));
    }
    // Minecraft names are 3–16 chars of [A-Za-z0-9_]; legacy accounts can be
    // shorter, so only bound the length and charset loosely.
    if name.is_empty()
        || name.len() > 32
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(StoreError::Invalid("invalid player name".into()));
    }
    Ok(())
}

const COLS: &str = "uuid, name, added_at, last_used_at, is_active";

impl Store {
    /// Add or update an account after sign-in and make it the active one.
    pub fn upsert_active_account(&self, uuid: &str, name: &str) -> Result<AccountRecord> {
        validate(uuid, name)?;
        let uuid = uuid.to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("UPDATE accounts SET is_active = 0 WHERE is_active = 1", [])?;
        tx.execute(
            "INSERT INTO accounts (uuid, name, added_at, last_used_at, is_active) VALUES (?1, ?2, ?3, ?3, 1)
             ON CONFLICT(uuid) DO UPDATE SET name = excluded.name, last_used_at = excluded.last_used_at, is_active = 1",
            params![uuid, name, now_unix()],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_account(&uuid)
    }

    pub fn get_account(&self, uuid: &str) -> Result<AccountRecord> {
        self.lock()
            .query_row(
                &format!("SELECT {COLS} FROM accounts WHERE uuid = ?1"),
                [uuid.to_ascii_lowercase()],
                from_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound)
    }

    /// Active account first, then most recently used.
    pub fn list_accounts(&self) -> Result<Vec<AccountRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM accounts ORDER BY is_active DESC, last_used_at DESC, added_at DESC"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn active_account(&self) -> Result<Option<AccountRecord>> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {COLS} FROM accounts WHERE is_active = 1"),
                [],
                from_row,
            )
            .optional()?)
    }

    pub fn set_active_account(&self, uuid: &str) -> Result<AccountRecord> {
        let uuid = uuid.to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE uuid = ?1)",
            [&uuid],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        tx.execute("UPDATE accounts SET is_active = 0 WHERE is_active = 1", [])?;
        tx.execute(
            "UPDATE accounts SET is_active = 1, last_used_at = ?2 WHERE uuid = ?1",
            params![uuid, now_unix()],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_account(&uuid)
    }

    /// Remove an account. If it was active, the most recently used remaining
    /// account becomes active. Returns the new active account, if any.
    pub fn remove_account(&self, uuid: &str) -> Result<Option<AccountRecord>> {
        let uuid = uuid.to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let was_active: Option<i64> = tx
            .query_row(
                "SELECT is_active FROM accounts WHERE uuid = ?1",
                [&uuid],
                |r| r.get(0),
            )
            .optional()?;
        let Some(was_active) = was_active else {
            return Err(StoreError::NotFound);
        };
        tx.execute("DELETE FROM accounts WHERE uuid = ?1", [&uuid])?;
        if was_active == 1 {
            tx.execute(
                "UPDATE accounts SET is_active = 1 WHERE uuid = (
                    SELECT uuid FROM accounts ORDER BY last_used_at DESC, added_at DESC LIMIT 1)",
                [],
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.active_account()
    }
}
