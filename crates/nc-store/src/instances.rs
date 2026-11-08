use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError, now_unix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    Vanilla,
    Fabric,
    Quilt,
    Neoforge,
}

impl Loader {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::Neoforge => "neoforge",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "vanilla" => Some(Self::Vanilla),
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "neoforge" => Some(Self::Neoforge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub dir_name: String,
    pub game_version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
    pub max_memory_mb: Option<u32>,
    pub java_path: Option<String>,
    pub jvm_args: Vec<String>,
    pub created_at: i64,
    pub last_played_at: Option<i64>,
    pub play_time_seconds: i64,
    /// Custom game directory; `None` = `<data>/instances/<dir_name>`.
    pub game_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewInstance {
    pub name: String,
    pub game_version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
    pub max_memory_mb: Option<u32>,
}

/// Partial update; `None` leaves a field unchanged.
#[derive(Debug, Clone, Default)]
pub struct InstancePatch {
    pub name: Option<String>,
    /// `Some(None)` resets to the launcher default.
    pub max_memory_mb: Option<Option<u32>>,
    /// `Some(None)` returns to the official Mojang runtime. The path must be
    /// validated (inspected) by the caller before saving.
    pub java_path: Option<Option<String>>,
    /// Already validated by the caller (`nc_core::jvmargs`).
    pub jvm_args: Option<Vec<String>>,
    /// New loader version (same loader). Validated by the caller.
    pub loader_version: Option<String>,
    /// `Some(None)` returns to the default folder. Validated by the caller.
    pub game_dir: Option<Option<String>>,
}

const COLUMNS: &str = "id, name, dir_name, game_version, loader, loader_version, max_memory_mb, java_path, jvm_args, created_at, last_played_at, play_time_seconds, game_dir";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Instance> {
    let loader: String = r.get(4)?;
    let jvm: String = r.get(8)?;
    Ok(Instance {
        id: r.get(0)?,
        name: r.get(1)?,
        dir_name: r.get(2)?,
        game_version: r.get(3)?,
        loader: Loader::parse(&loader).unwrap_or(Loader::Vanilla),
        loader_version: r.get(5)?,
        max_memory_mb: r.get(6)?,
        java_path: r.get(7)?,
        // Corrupt JSON degrades to no extra args rather than failing the list.
        jvm_args: serde_json::from_str(&jvm).unwrap_or_default(),
        created_at: r.get(9)?,
        last_played_at: r.get(10)?,
        play_time_seconds: r.get(11)?,
        game_dir: r.get(12)?,
    })
}

/// Trim and validate a display name.
pub fn clean_name(name: &str) -> Result<String> {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    if n.is_empty() || n.chars().count() > 64 {
        return Err(StoreError::Invalid("name must be 1–64 characters".into()));
    }
    Ok(n)
}

fn validate_memory(mb: Option<u32>) -> Result<()> {
    match mb {
        Some(m) if !(512..=65536).contains(&m) => Err(StoreError::Invalid(
            "memory must be between 512 and 65536 MiB".into(),
        )),
        _ => Ok(()),
    }
}

/// Folder name derived from the display name: ASCII letters, digits, `-`,
/// `_`, `.` and spaces; everything else becomes `-`. Always a valid,
/// non-reserved Windows path segment.
pub fn slug(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s: String = s.chars().take(48).collect();
    let s = s
        .trim_matches(|c: char| c == '.' || c == ' ' || c == '-')
        .to_owned();
    if s.is_empty() {
        return "instance".into();
    }
    if nc_core::paths::validate_segment(&s).is_err() {
        return format!("{s}-1");
    }
    s
}

impl Store {
    /// Create an instance. `folder_taken` lets the caller also reject folder
    /// names that already exist on disk.
    pub fn create_instance(
        &self,
        new: NewInstance,
        folder_taken: impl Fn(&str) -> bool,
    ) -> Result<Instance> {
        let name = clean_name(&new.name)?;
        validate_memory(new.max_memory_mb)?;
        nc_core::paths::validate_segment(&new.game_version)
            .map_err(|_| StoreError::Invalid("invalid game version".into()))?;

        let conn = self.lock();
        let base = slug(&name);
        let mut dir = base.clone();
        let mut n = 2;
        loop {
            let in_db: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM instances WHERE dir_name = ?1)",
                [&dir],
                |r| r.get(0),
            )?;
            if !in_db && !folder_taken(&dir) {
                break;
            }
            dir = format!("{base}-{n}");
            n += 1;
            if n > 1000 {
                return Err(StoreError::Invalid(
                    "could not find a free folder name".into(),
                ));
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO instances (id, name, dir_name, game_version, loader, loader_version, max_memory_mb, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, dir, new.game_version, new.loader.as_str(), new.loader_version, new.max_memory_mb, now_unix()],
        )?;
        drop(conn);
        self.get_instance(&id)
    }

    pub fn list_instances(&self) -> Result<Vec<Instance>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM instances ORDER BY last_played_at IS NULL, last_played_at DESC, created_at DESC"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_instance(&self, id: &str) -> Result<Instance> {
        self.lock()
            .query_row(
                &format!("SELECT {COLUMNS} FROM instances WHERE id = ?1"),
                [id],
                from_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound)
    }

    pub fn update_instance(&self, id: &str, patch: InstancePatch) -> Result<Instance> {
        let conn = self.lock();
        if let Some(name) = patch.name {
            let name = clean_name(&name)?;
            if conn.execute(
                "UPDATE instances SET name = ?1 WHERE id = ?2",
                params![name, id],
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        if let Some(mem) = patch.max_memory_mb {
            validate_memory(mem)?;
            if conn.execute(
                "UPDATE instances SET max_memory_mb = ?1 WHERE id = ?2",
                params![mem, id],
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        if let Some(java) = patch.java_path {
            if java
                .as_ref()
                .is_some_and(|p| p.is_empty() || p.len() > 1024)
            {
                return Err(StoreError::Invalid("invalid Java path".into()));
            }
            if conn.execute(
                "UPDATE instances SET java_path = ?1 WHERE id = ?2",
                params![java, id],
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        if let Some(args) = patch.jvm_args {
            if args.len() > 64 || args.iter().any(|a| a.len() > 512) {
                return Err(StoreError::Invalid(
                    "too many or too long JVM arguments".into(),
                ));
            }
            let json =
                serde_json::to_string(&args).map_err(|e| StoreError::Invalid(e.to_string()))?;
            if conn.execute(
                "UPDATE instances SET jvm_args = ?1 WHERE id = ?2",
                params![json, id],
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        if let Some(lv) = patch.loader_version {
            if lv.is_empty() || lv.len() > 64 {
                return Err(StoreError::Invalid("invalid loader version".into()));
            }
            if conn.execute(
                "UPDATE instances SET loader_version = ?1 WHERE id = ?2 AND loader != 'vanilla'",
                params![lv, id],
            )? == 0
            {
                return Err(StoreError::Invalid("this instance has no loader".into()));
            }
        }
        if let Some(dir) = patch.game_dir {
            if dir.as_ref().is_some_and(|d| d.len() < 3 || d.len() > 1024) {
                return Err(StoreError::Invalid("invalid game folder".into()));
            }
            if conn.execute(
                "UPDATE instances SET game_dir = ?1 WHERE id = ?2",
                params![dir, id],
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        drop(conn);
        self.get_instance(id)
    }

    /// Remove the row and return it so the caller can deal with the folder.
    pub fn delete_instance(&self, id: &str) -> Result<Instance> {
        let inst = self.get_instance(id)?;
        self.lock()
            .execute("DELETE FROM instances WHERE id = ?1", [id])?;
        Ok(inst)
    }

    pub fn record_play(&self, id: &str, started_at: i64, seconds: i64) -> Result<()> {
        self.lock().execute(
            "UPDATE instances SET last_played_at = ?1, play_time_seconds = play_time_seconds + ?2 WHERE id = ?3",
            params![started_at, seconds.max(0), id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(slug("My Survival World"), "My Survival World");
        assert_eq!(slug("../../evil"), "evil");
        assert_eq!(slug("a/b\\c:d*e"), "a-b-c-d-e");
        assert_eq!(slug("Ночь"), "instance");
        assert_eq!(slug("CON"), "CON-1");
        assert_eq!(slug("trailing. "), "trailing");
        assert_eq!(slug(&"x".repeat(100)).len(), 48);
        for s in ["My Survival World", "../../evil", "CON", "Ночь", "a:b"] {
            assert!(nc_core::paths::validate_segment(&slug(s)).is_ok(), "{s}");
        }
    }
}
