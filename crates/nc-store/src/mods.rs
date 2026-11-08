use rusqlite::{Row, params};
use serde::Serialize;

use crate::{Result, Store, StoreError, now_unix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModSource {
    Modrinth,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRecord {
    pub instance_id: String,
    pub file_name: String,
    pub source: ModSource,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub title: String,
    pub version_number: Option<String>,
    pub icon_url: Option<String>,
    pub sha1: String,
    pub sha512: String,
    pub size: i64,
    pub enabled: bool,
    pub required_by: Option<String>,
    pub installed_at: i64,
}

const COLS: &str = "instance_id, file_name, source, project_id, version_id, title, version_number, icon_url, sha1, sha512, size, enabled, required_by, installed_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<ModRecord> {
    let source: String = r.get(2)?;
    Ok(ModRecord {
        instance_id: r.get(0)?,
        file_name: r.get(1)?,
        source: if source == "modrinth" {
            ModSource::Modrinth
        } else {
            ModSource::Local
        },
        project_id: r.get(3)?,
        version_id: r.get(4)?,
        title: r.get(5)?,
        version_number: r.get(6)?,
        icon_url: r.get(7)?,
        sha1: r.get(8)?,
        sha512: r.get(9)?,
        size: r.get(10)?,
        enabled: r.get::<_, i64>(11)? == 1,
        required_by: r.get(12)?,
        installed_at: r.get(13)?,
    })
}

impl Store {
    pub fn list_mods(&self, instance_id: &str) -> Result<Vec<ModRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM instance_mods WHERE instance_id = ?1 ORDER BY lower(title)"
        ))?;
        let rows = stmt.query_map([instance_id], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Insert or replace the row for `(instance_id, file_name)`.
    pub fn upsert_mod(&self, m: &ModRecord) -> Result<()> {
        if m.title.is_empty() || m.title.len() > 256 || m.sha512.len() != 128 || m.sha1.len() != 40
        {
            return Err(StoreError::Invalid("invalid mod record".into()));
        }
        let installed_at = if m.installed_at == 0 {
            now_unix()
        } else {
            m.installed_at
        };
        self.lock().execute(
            &format!("INSERT OR REPLACE INTO instance_mods ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
            params![
                m.instance_id,
                m.file_name,
                if m.source == ModSource::Modrinth { "modrinth" } else { "local" },
                m.project_id,
                m.version_id,
                m.title,
                m.version_number,
                m.icon_url,
                m.sha1,
                m.sha512,
                m.size,
                m.enabled as i64,
                m.required_by,
                installed_at
            ],
        )?;
        Ok(())
    }

    pub fn set_mod_enabled(&self, instance_id: &str, file_name: &str, enabled: bool) -> Result<()> {
        let n = self.lock().execute(
            "UPDATE instance_mods SET enabled = ?3 WHERE instance_id = ?1 AND file_name = ?2",
            params![instance_id, file_name, enabled as i64],
        )?;
        if n == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    pub fn remove_mod(&self, instance_id: &str, file_name: &str) -> Result<()> {
        self.lock().execute(
            "DELETE FROM instance_mods WHERE instance_id = ?1 AND file_name = ?2",
            params![instance_id, file_name],
        )?;
        Ok(())
    }
}
