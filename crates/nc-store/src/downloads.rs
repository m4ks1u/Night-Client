use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError};

/// Rows kept; older ones are pruned on insert.
pub const HISTORY_LIMIT: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Completed,
    Failed,
    Cancelled,
}

impl DownloadStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    /// Assigned by the database; ignored on insert.
    pub id: i64,
    pub kind: String,
    pub target: String,
    pub status: DownloadStatus,
    pub repair: bool,
    pub queued_at: i64,
    pub started_at: i64,
    pub finished_at: i64,
    pub files_downloaded: i64,
    pub files_verified: i64,
    pub bytes_downloaded: i64,
    pub repaired: Vec<String>,
    pub error: Option<String>,
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<DownloadRecord> {
    let status: String = r.get(3)?;
    let repaired: String = r.get(11)?;
    Ok(DownloadRecord {
        id: r.get(0)?,
        kind: r.get(1)?,
        target: r.get(2)?,
        status: DownloadStatus::parse(&status),
        repair: r.get::<_, i64>(4)? != 0,
        queued_at: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        files_downloaded: r.get(8)?,
        files_verified: r.get(9)?,
        bytes_downloaded: r.get(10)?,
        repaired: serde_json::from_str(&repaired).unwrap_or_default(),
        error: r.get(12)?,
    })
}

impl Store {
    pub fn record_download(&self, rec: &DownloadRecord) -> Result<i64> {
        if rec.kind.is_empty() || rec.kind.len() > 32 || rec.target.len() > 256 {
            return Err(StoreError::Invalid("invalid download record".into()));
        }
        let repaired: Vec<&String> = rec.repaired.iter().take(200).collect();
        let repaired =
            serde_json::to_string(&repaired).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let error = rec
            .error
            .as_ref()
            .map(|e| e.chars().take(2000).collect::<String>());
        let conn = self.lock();
        conn.execute(
            "INSERT INTO download_history (kind, target, status, repair, queued_at, started_at, finished_at,
                files_downloaded, files_verified, bytes_downloaded, repaired, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                rec.kind,
                rec.target,
                rec.status.as_str(),
                rec.repair as i64,
                rec.queued_at,
                rec.started_at,
                rec.finished_at,
                rec.files_downloaded,
                rec.files_verified,
                rec.bytes_downloaded,
                repaired,
                error
            ],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "DELETE FROM download_history WHERE id NOT IN (SELECT id FROM download_history ORDER BY id DESC LIMIT ?1)",
            [HISTORY_LIMIT],
        )?;
        Ok(id)
    }

    pub fn list_downloads(&self, limit: u32) -> Result<Vec<DownloadRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, kind, target, status, repair, queued_at, started_at, finished_at,
                    files_downloaded, files_verified, bytes_downloaded, repaired, error
             FROM download_history ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit.min(HISTORY_LIMIT as u32)], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn clear_downloads(&self) -> Result<usize> {
        Ok(self.lock().execute("DELETE FROM download_history", [])?)
    }
}
