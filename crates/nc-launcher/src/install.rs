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
            });
            Some(dest)
        }
        None => None,
    };

    if assets.is_virtual || assets.map_to_resources {
        // Pre-1.7 asset layouts need an extra copy step; out of Stage 0 scope.
        tracing::warn!(%id, "legacy virtual asset layout is not supported yet");
    }

    let files = {
        let e = emit(Phase::GameFiles);
        meta.dl.fetch_all(tasks, cancel, e).await?
    };

    let marker = layout.installed_marker(id)?;
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

    Ok(Installed {
        version,
        libraries: libs,
        java,
        client_jar,
        log_config,
        files,
    })
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
