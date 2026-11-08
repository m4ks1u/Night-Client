//! Vanilla installation: version JSON, client jar, libraries, assets,
//! logging configuration and the matching official Java runtime.

use std::{path::PathBuf, sync::Arc};

use nc_core::{
    java,
    libraries::{self, ResolvedLibrary},
    manifest::VersionJson,
    rules::Environment,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    LauncherError,
    download::{DownloadTask, Progress, Summary},
    fsutil,
    meta::Meta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Metadata,
    Runtime,
    GameFiles,
    /// Loader files and (NeoForge) installer processors.
    Loader,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub phase: Phase,
    #[serde(flatten)]
    pub progress: Progress,
}

pub type ProgressFn = Arc<dyn Fn(InstallProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct Installed {
    pub version: VersionJson,
    pub libraries: Vec<ResolvedLibrary>,
    pub java: PathBuf,
    pub client_jar: PathBuf,
    pub log_config: Option<PathBuf>,
    pub files: Summary,
    /// The version was already marked installed before this run (a repair).
    pub was_installed: bool,
    /// Files that were missing or damaged and were re-downloaded during a
    /// repair, relative to the data directory (at most 200 listed).
    pub repaired: Vec<String>,
    /// Number of files re-downloaded during a repair (may exceed the list).
    pub repaired_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    id: String,
    launcher_version: String,
}

/// Install (or repair) a vanilla version. Safe to call repeatedly: files that
/// already verify are skipped, so a second run downloads nothing.
pub async fn install_vanilla(
    meta: &Meta,
    id: &str,
    env: &Environment,
    cancel: &CancellationToken,
    on_progress: ProgressFn,
) -> Result<Installed, LauncherError> {
    let layout = &meta.layout;
    let emit = |phase: Phase| {
        let f = on_progress.clone();
        move |progress: Progress| f(InstallProgress { phase, progress })
    };
    emit(Phase::Metadata)(Progress::default());

    let (manifest, _) = meta.version_manifest(cancel).await?;
    let entry = manifest
        .find(id)
        .ok_or_else(|| LauncherError::VersionNotFound(id.to_owned()))?;
    let version = meta.version_json(entry, cancel).await?;
    let asset_ref =
        version
            .asset_index
            .clone()
            .ok_or_else(|| nc_core::CoreError::MissingField {
                version: id.to_owned(),
                field: "assetIndex",
            })?;
    let assets = meta.asset_index(&asset_ref, cancel).await?;

    // From here on the version is "being installed": drop the marker so an
    // interrupted repair can never leave a version that looks complete.
    let marker = layout.installed_marker(id)?;
    let was_installed = marker.is_file();
    if was_installed {
        tokio::fs::remove_file(&marker)
            .await
            .map_err(LauncherError::io(&marker))?;
    }

    // Java first: it is needed to launch and is the second-largest download.
    let req = java::requirement(&version);
    let java = {
        let e = emit(Phase::Runtime);
        crate::java::ensure_runtime(meta, &req.component, cancel, Arc::new(e)).await?
    };

    let libs = libraries::resolve(&version, env)?;
    let mut tasks = Vec::with_capacity(libs.len() + assets.objects.len() + 2);

    let client = version
        .downloads
        .as_ref()
        .and_then(|d| d.client.as_ref())
        .ok_or_else(|| nc_core::CoreError::MissingField {
            version: id.to_owned(),
            field: "downloads.client",
        })?;
    let client_jar = layout.client_jar(id)?;
    tasks.push(DownloadTask {
        url: client.url.clone(),
        dest: client_jar.clone(),
        sha1: Some(client.sha1.clone()),
        size: Some(client.size),
        root: None,
    });

    for lib in &libs {
        let url = lib
            .url
            .clone()
            .ok_or_else(|| nc_core::CoreError::MissingField {
                version: id.to_owned(),
                field: "library download url",
            })?;
        tasks.push(DownloadTask {
            url,
            dest: fsutil::safe_join(&layout.libraries_dir(), &lib.path)?,
            sha1: lib.sha1.clone(),
            size: lib.size,
            root: None,
        });
    }

    let objects_root = layout.assets_dir();
    for obj in assets.objects.values() {
        let (Some(path), Some(url)) = (obj.object_path(), obj.url()) else {
            return Err(nc_core::CoreError::UnsafePath(obj.hash.clone()).into());
        };
        tasks.push(DownloadTask {
            url,
            dest: fsutil::safe_join(&objects_root, &path)?,
            sha1: Some(obj.hash.clone()),
            size: Some(obj.size),
            root: None,
        });
    }

    let log_config = match version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        Some(cfg) => {
            let dest = layout.log_config(&cfg.file.id)?;
            tasks.push(DownloadTask {
                url: cfg.file.url.clone(),
                dest: dest.clone(),
                sha1: Some(cfg.file.sha1.clone()),
                size: Some(cfg.file.size),
                root: None,
            });
            Some(dest)
        }
        None => None,
    };

    let files = {
        let e = emit(Phase::GameFiles);
        meta.dl.fetch_all(tasks, cancel, e).await?
    };

    // Pre-1.7 "virtual" indexes (e.g. `legacy` for 1.6.x) expect the objects
    // under their real names in assets/virtual/<index>. Pre-1.6
    // (`map_to_resources`) copies go into each instance at launch instead.
    if assets.is_virtual {
        let dest = layout.assets_dir().join("virtual").join(&asset_ref.id);
        nc_core::paths::validate_segment(&asset_ref.id)?;
        let root = layout.assets_dir();
        let idx = assets.clone();
        let n = tokio::task::spawn_blocking(move || materialize_assets(&idx, &root, &dest))
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))??;
        tracing::info!(%id, files = n, "virtual assets laid out");
    }

    let body = serde_json::to_vec(&Marker {
        id: id.to_owned(),
        launcher_version: env!("CARGO_PKG_VERSION").into(),
    })
    .map_err(LauncherError::parse("install marker"))?;
    let m = marker.clone();
    tokio::task::spawn_blocking(move || fsutil::atomic_write(&m, &body))
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))?
        .map_err(LauncherError::io(marker))?;

    let root = layout.root().to_path_buf();
    let (repaired, repaired_count) = if was_installed {
        let list = files
            .fetched
            .iter()
            .take(200)
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        (list, files.downloaded_files)
    } else {
        (Vec::new(), 0)
    };
    if was_installed {
        tracing::info!(%id, repaired = repaired_count, "verification finished");
    }

    Ok(Installed {
        version,
        libraries: libs,
        java,
        client_jar,
        log_config,
        files,
        was_installed,
        repaired,
        repaired_count,
    })
}

/// Place asset objects under their logical names (`<dest>/<name>`), hard-linking
/// when possible and copying otherwise. Names come from the (hash-verified)
/// index but are still validated as relative paths. Returns files written.
pub fn materialize_assets(
    index: &nc_core::manifest::AssetIndex,
    assets_root: &std::path::Path,
    dest: &std::path::Path,
) -> Result<u64, LauncherError> {
    let mut written = 0;
    for (name, obj) in &index.objects {
        let src = obj
            .object_path()
            .map(|p| fsutil::safe_join(assets_root, &p))
            .ok_or_else(|| nc_core::CoreError::UnsafePath(obj.hash.clone()))??;
        let target = fsutil::safe_join(dest, name)?;
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == obj.size) {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(LauncherError::io(parent))?;
        }
        let _ = std::fs::remove_file(&target);
        if std::fs::hard_link(&src, &target).is_err() {
            std::fs::copy(&src, &target).map_err(LauncherError::io(&target))?;
        }
        written += 1;
    }
    Ok(written)
}

/// Extract legacy natives jars (pre-1.19) into `natives_dir`.
pub async fn prepare_natives(
    libraries_dir: PathBuf,
    libs: Vec<ResolvedLibrary>,
    natives_dir: PathBuf,
) -> Result<(), LauncherError> {
    tokio::task::spawn_blocking(move || -> Result<(), LauncherError> {
        std::fs::create_dir_all(&natives_dir).map_err(LauncherError::io(&natives_dir))?;
        for lib in libs {
            if let nc_core::libraries::LibraryKind::Native { exclude } = &lib.kind {
                let jar = fsutil::safe_join(&libraries_dir, &lib.path)?;
                fsutil::extract_zip(&jar, &natives_dir, exclude, 512 * 1024 * 1024)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))?
}
