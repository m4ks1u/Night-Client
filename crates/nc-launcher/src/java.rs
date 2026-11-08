//! Official Mojang Java runtimes ("java-runtime-delta", "java-runtime-epsilon", ...).
//!
//! Each runtime is a per-file manifest with SHA-1 hashes, so it goes through
//! the same verified download engine as game files. No third-party JDK
//! vendor or archive extraction is involved.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{
    LauncherError,
    download::{DownloadTask, Progress},
    fsutil,
    meta::Meta,
};

pub const RUNTIME_INDEX_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const PLATFORM: &str = "windows-x64";

#[derive(Debug, Deserialize)]
struct RuntimeEntry {
    manifest: ManifestRef,
    version: RuntimeVersion,
}

#[derive(Debug, Deserialize)]
struct ManifestRef {
    sha1: String,
    size: u64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeManifest {
    pub files: BTreeMap<String, RuntimeFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RuntimeFile {
    Directory,
    File { downloads: RuntimeDownloads },
    Link { target: String },
}

#[derive(Debug, Deserialize)]
pub struct RuntimeDownloads {
    pub raw: RawDownload,
}

#[derive(Debug, Deserialize)]
pub struct RawDownload {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

/// Turn a runtime manifest into verified download tasks rooted at `dir`.
/// Every path is validated, so a malicious manifest cannot write outside it.
pub fn plan(
    manifest: &RuntimeManifest,
    dir: &std::path::Path,
) -> Result<(Vec<PathBuf>, Vec<DownloadTask>), LauncherError> {
    let mut dirs = Vec::new();
    let mut tasks = Vec::new();
    for (path, file) in &manifest.files {
        let target = fsutil::safe_join(dir, path)?;
        match file {
            RuntimeFile::Directory => dirs.push(target),
            RuntimeFile::File { downloads } => tasks.push(DownloadTask {
                url: downloads.raw.url.clone(),
                dest: target,
                sha1: Some(downloads.raw.sha1.clone()),
                size: Some(downloads.raw.size),
                root: None,
            }),
            // Windows runtimes contain no links; never follow them.
            RuntimeFile::Link { target: t } => {
                tracing::debug!(%path, link = %t, "skipping runtime link")
            }
        }
    }
    Ok((dirs, tasks))
}

/// Make sure `component` is installed and return the path to `javaw.exe`.
pub async fn ensure_runtime(
    meta: &Meta,
    component: &str,
    cancel: &CancellationToken,
    on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
) -> Result<PathBuf, LauncherError> {
    let dir = meta.layout.runtime_dir(component)?;
    let javaw = dir.join("bin").join("javaw.exe");

    let index: BTreeMap<String, BTreeMap<String, Vec<RuntimeEntry>>> = {
        let bytes = meta.dl.get_bytes(RUNTIME_INDEX_URL, cancel).await?;
        serde_json::from_slice(&bytes).map_err(LauncherError::parse("Java runtime index"))?
    };
    let entry = index
        .get(PLATFORM)
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .ok_or_else(|| LauncherError::RuntimeUnavailable(component.to_owned()))?;

    let manifest_path = meta
        .layout
        .runtimes_dir()
        .join(format!("{component}.manifest.json"));
    meta.dl
        .fetch_all(
            vec![DownloadTask {
                url: entry.manifest.url.clone(),
                dest: manifest_path.clone(),
                sha1: Some(entry.manifest.sha1.clone()),
                size: Some(entry.manifest.size),
                root: None,
            }],
            cancel,
            |_| {},
        )
        .await?;
    let manifest: RuntimeManifest =
        crate::meta::read_json(&manifest_path, "Java runtime manifest").await?;

    let (dirs, tasks) = plan(&manifest, &dir)?;
    for d in dirs {
        tokio::fs::create_dir_all(&d)
            .await
            .map_err(LauncherError::io(&d))?;
    }
    meta.dl
        .fetch_all(tasks, cancel, move |p| on_progress(p))
        .await?;

    if !javaw.is_file() {
        return Err(LauncherError::RuntimeUnavailable(format!(
            "{component}: javaw.exe missing after install"
        )));
    }
    let marker = dir.join(".version");
    let v = entry.version.name.clone();
    tokio::task::spawn_blocking(move || fsutil::atomic_write(&marker, v.as_bytes()))
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))?
        .map_err(LauncherError::io(&dir))?;
    Ok(javaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_rejects_escaping_paths() {
        let m: RuntimeManifest = serde_json::from_str(
            r#"{"files":{"../../evil.exe":{"type":"file","downloads":{"raw":{"sha1":"a","size":1,"url":"https://x"}}}}}"#,
        )
        .unwrap();
        assert!(plan(&m, std::path::Path::new(r"C:\rt")).is_err());
    }

    #[test]
    fn plan_maps_files_and_dirs() {
        let m: RuntimeManifest = serde_json::from_str(
            r#"{"files":{
                "bin":{"type":"directory"},
                "bin/javaw.exe":{"type":"file","executable":true,"downloads":{"raw":{"sha1":"abc","size":3,"url":"https://x/javaw.exe"}}},
                "lib/link":{"type":"link","target":"../x"}
            }}"#,
        )
        .unwrap();
        let (dirs, tasks) = plan(&m, std::path::Path::new(r"C:\rt")).unwrap();
        assert_eq!(dirs, vec![PathBuf::from(r"C:\rt\bin")]);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].dest, PathBuf::from(r"C:\rt\bin\javaw.exe"));
    }
}
