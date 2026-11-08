//! The Modrinth modpack an instance was made from, and the files it put
//! there (migration v8).

use rusqlite::params;
use serde::Serialize;

use crate::{Result, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackRecord {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub name: String,
    pub version_number: String,
    pub icon_url: Option<String>,
    pub installed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFileRecord {
    pub path: String,
    /// Lower-case hex SHA-1 at install time.
    pub sha1: String,
    pub is_override: bool,
}

impl Store {
    /// Replace an instance's modpack record and file list in one transaction.
    pub fn set_modpack(
        &self,
        instance_id: &str,
        pack: &ModpackRecord,
        files: &[PackFileRecord],
    ) -> Result<()> {
        if pack.name.is_empty() || pack.name.len() > 200 || pack.version_number.is_empty() {
            return Err(StoreError::Invalid(
                "invalid modpack name or version".into(),
            ));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM instance_modpack WHERE instance_id = ?1",
            params![instance_id],
        )?;
        tx.execute(
            "INSERT INTO instance_modpack (instance_id, project_id, version_id, name, version_number, icon_url, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                instance_id,
                pack.project_id,
                pack.version_id,
                pack.name,
                pack.version_number.chars().take(200).collect::<String>(),
                pack.icon_url,
                pack.installed_at
            ],
        )?;
        {
            let mut ins = tx.prepare(
                "INSERT OR REPLACE INTO instance_modpack_files (instance_id, path, sha1, is_override)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for f in files {
                ins.execute(params![
                    instance_id,
                    f.path,
                    f.sha1.to_ascii_lowercase(),
                    i64::from(f.is_override)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_modpack(&self, instance_id: &str) -> Result<Option<ModpackRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT project_id, version_id, name, version_number, icon_url, installed_at
             FROM instance_modpack WHERE instance_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![instance_id], |r| {
            Ok(ModpackRecord {
                project_id: r.get(0)?,
                version_id: r.get(1)?,
                name: r.get(2)?,
                version_number: r.get(3)?,
                icon_url: r.get(4)?,
                installed_at: r.get(5)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn modpack_files(&self, instance_id: &str) -> Result<Vec<PackFileRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT path, sha1, is_override FROM instance_modpack_files
             WHERE instance_id = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map(params![instance_id], |r| {
            Ok(PackFileRecord {
                path: r.get(0)?,
                sha1: r.get(1)?,
                is_override: r.get::<_, i64>(2)? == 1,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
