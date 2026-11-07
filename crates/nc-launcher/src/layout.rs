//! On-disk layout of the launcher data directory.
//!
//! ```text
//! <root>/                 default %APPDATA%\NightClient
//!   cache/                manifests and other refreshable metadata
//!   versions/<id>/        <id>.json, <id>.jar, .installed marker
//!   libraries/            shared Maven-layout library cache
//!   assets/               shared indexes/, objects/, log_configs/
//!   runtimes/<component>/ official Mojang Java runtimes
//!   instances/<name>/     independent game directories
//! ```

use std::path::{Path, PathBuf};

use nc_core::{CoreError, paths::validate_segment};

#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `%APPDATA%\NightClient`, falling back to the current directory.
    pub fn default_root() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("NightClient")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }
    pub fn runtimes_dir(&self) -> PathBuf {
        self.root.join("runtimes")
    }
    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }

    pub fn version_dir(&self, id: &str) -> Result<PathBuf, CoreError> {
        validate_segment(id)?;
        Ok(self.root.join("versions").join(id))
    }
    pub fn version_json(&self, id: &str) -> Result<PathBuf, CoreError> {
        Ok(self.version_dir(id)?.join(format!("{id}.json")))
    }
    pub fn client_jar(&self, id: &str) -> Result<PathBuf, CoreError> {
        Ok(self.version_dir(id)?.join(format!("{id}.jar")))
    }
    pub fn installed_marker(&self, id: &str) -> Result<PathBuf, CoreError> {
        Ok(self.version_dir(id)?.join(".installed"))
    }
    pub fn asset_index(&self, id: &str) -> Result<PathBuf, CoreError> {
        validate_segment(id)?;
        Ok(self.assets_dir().join("indexes").join(format!("{id}.json")))
    }
    pub fn log_config(&self, file_id: &str) -> Result<PathBuf, CoreError> {
        validate_segment(file_id)?;
        Ok(self.assets_dir().join("log_configs").join(file_id))
    }
    pub fn runtime_dir(&self, component: &str) -> Result<PathBuf, CoreError> {
        validate_segment(component)?;
        Ok(self.runtimes_dir().join(component))
    }
    pub fn instance_dir(&self, name: &str) -> Result<PathBuf, CoreError> {
        validate_segment(name)?;
        Ok(self.instances_dir().join(name))
    }
    /// Per-instance, per-version natives directory (never shared between
    /// running games).
    pub fn natives_dir(&self, instance: &str, version: &str) -> Result<PathBuf, CoreError> {
        validate_segment(version)?;
        Ok(self.instance_dir(instance)?.join(".natives").join(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_in_ids() {
        let l = Layout::new(r"C:\NC");
        assert!(l.version_dir("1.21.1").is_ok());
        assert!(l.version_dir("26.4-snapshot-7").is_ok());
        for bad in ["..", "../x", "a/b", "C:x", "con"] {
            assert!(l.version_dir(bad).is_err(), "{bad}");
            assert!(l.instance_dir(bad).is_err(), "{bad}");
        }
    }
}
