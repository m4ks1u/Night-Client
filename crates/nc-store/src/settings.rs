use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{Result, Store, StoreError, now_unix};

/// Launcher-wide preferences. Missing fields take their defaults, so adding
/// a field never needs a migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    /// Memory for new instances (MiB).
    pub default_max_memory_mb: u32,
    /// Parallel downloads (1–16).
    pub download_concurrency: u8,
    /// Hide the launcher window while the game runs.
    pub hide_on_launch: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_max_memory_mb: 4096,
            download_concurrency: 8,
            hide_on_launch: false,
        }
    }
}

impl AppSettings {
    pub const KEY: &'static str = "app";

    pub fn validate(&self) -> Result<()> {
        if !(512..=65536).contains(&self.default_max_memory_mb) {
            return Err(StoreError::Invalid(
                "memory must be between 512 and 65536 MiB".into(),
            ));
        }
        if !(1..=16).contains(&self.download_concurrency) {
            return Err(StoreError::Invalid(
                "parallel downloads must be between 1 and 16".into(),
            ));
        }
        Ok(())
    }
}

impl Store {
    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        raw.map(|s| {
            serde_json::from_str(&s).map_err(|e| StoreError::Corrupt {
                key: key.into(),
                message: e.to_string(),
            })
        })
        .transpose()
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value).map_err(|e| StoreError::Invalid(e.to_string()))?;
        self.lock().execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            rusqlite::params![key, json, now_unix()],
        )?;
        Ok(())
    }

    /// App settings; a corrupt row falls back to defaults instead of
    /// preventing the launcher from starting.
    pub fn app_settings(&self) -> AppSettings {
        match self.get_setting::<AppSettings>(AppSettings::KEY) {
            Ok(Some(s)) if s.validate().is_ok() => s,
            Ok(_) => AppSettings::default(),
            Err(e) => {
                tracing::warn!(error = %e, "app settings unreadable, using defaults");
                AppSettings::default()
            }
        }
    }

    pub fn save_app_settings(&self, s: &AppSettings) -> Result<()> {
        s.validate()?;
        self.set_setting(AppSettings::KEY, s)
    }
}
