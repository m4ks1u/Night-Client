//! Mod loaders on top of vanilla: Fabric and Quilt (profile merge) and NeoForge
//! (installer processors). Every loader install first installs (or
//! verifies) its vanilla parent, then writes `versions/<launch id>/` with:
//!
//! * `<id>.json`: the loader's version JSON (with `inheritsFrom`);
//! * `<id>.jar`: a hard link/copy of the vanilla client jar. Loaders such as
//!   NeoForge identify the vanilla jar on the classpath by this name;
//! * `.installed`: written last, after every file verified.

pub mod fabric;
pub mod neoforge;

use std::path::Path;

use nc_core::{
    loader::{InstallKey, LoaderKind},
    manifest::VersionJson,
    profile,
    rules::Environment,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::{
    LauncherError, fsutil,
    install::{self, Installed, ProgressFn},
    layout::Layout,
    meta::{Meta, read_json},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

/// Install (or repair) whatever `key` describes.
pub async fn install(
    meta: &Meta,
    key: &InstallKey,
    env: &Environment,
    cancel: &CancellationToken,
    on_progress: ProgressFn,
) -> Result<Installed, LauncherError> {
    match key.kind {
        LoaderKind::Vanilla => {
            install::install_vanilla(meta, &key.minecraft, env, cancel, on_progress).await
        }
        LoaderKind::Fabric => {
            fabric::install(fabric::Flavor::Fabric, meta, key, env, cancel, on_progress).await
        }
        LoaderKind::Quilt => {
            fabric::install(fabric::Flavor::Quilt, meta, key, env, cancel, on_progress).await
        }
        LoaderKind::NeoForge => neoforge::install(meta, key, env, cancel, on_progress).await,
    }
}

/// Loader versions available for a Minecraft version, newest first.
pub async fn versions(
    meta: &Meta,
    kind: LoaderKind,
    minecraft: &str,
    cancel: &CancellationToken,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    match kind {
        LoaderKind::Vanilla => Ok(Vec::new()),
        LoaderKind::Fabric => {
            fabric::versions(fabric::Flavor::Fabric, meta, minecraft, cancel).await
        }
        LoaderKind::Quilt => fabric::versions(fabric::Flavor::Quilt, meta, minecraft, cancel).await,
        LoaderKind::NeoForge => neoforge::versions(meta, minecraft, cancel).await,
    }
}

/// Read `versions/<id>/<id>.json` and merge its `inheritsFrom` chain from
/// local files (at most 4 levels; cycles are impossible within that bound).
pub async fn resolve_version(layout: &Layout, id: &str) -> Result<VersionJson, LauncherError> {
    let mut v: VersionJson = read_json(&layout.version_json(id)?, &format!("version {id}")).await?;
    for _ in 0..4 {
        let Some(parent_id) = v.inherits_from.clone() else {
            return Ok(v);
        };
        let parent: VersionJson = read_json(
            &layout.version_json(&parent_id)?,
            &format!("version {parent_id}"),
        )
        .await?;
        v = profile::merge(v, parent)?;
    }
    Err(nc_core::CoreError::MissingField {
        version: id.to_owned(),
        field: "inheritsFrom chain shorter than 5",
    }
    .into())
}

/// Remove the marker before touching files (see `install_vanilla`).
pub(crate) async fn begin(layout: &Layout, id: &str) -> Result<bool, LauncherError> {
    let marker = layout.installed_marker(id)?;
    let was = marker.is_file();
    if was {
        tokio::fs::remove_file(&marker)
            .await
            .map_err(LauncherError::io(&marker))?;
    }
    Ok(was)
}

/// Write the version JSON and the client jar link for `id`, then the marker.
pub(crate) async fn finish(
    layout: &Layout,
    id: &str,
    json: &[u8],
    vanilla_jar: &Path,
) -> Result<(), LauncherError> {
    let (layout, id, json, src) = (
        layout.clone(),
        id.to_owned(),
        json.to_vec(),
        vanilla_jar.to_path_buf(),
    );
    tokio::task::spawn_blocking(move || -> Result<(), LauncherError> {
        let json_path = layout.version_json(&id)?;
        fsutil::atomic_write(&json_path, &json).map_err(LauncherError::io(&json_path))?;
        let jar = layout.client_jar(&id)?;
        let same_size = std::fs::metadata(&jar).ok().map(|m| m.len())
            == std::fs::metadata(&src).ok().map(|m| m.len());
        if !same_size || fsutil::sha1_file(&jar).ok() != fsutil::sha1_file(&src).ok() {
            let _ = std::fs::remove_file(&jar);
            if std::fs::hard_link(&src, &jar).is_err() {
                std::fs::copy(&src, &jar).map_err(LauncherError::io(&jar))?;
            }
        }
        let marker = layout.installed_marker(&id)?;
        let body = format!(
            "{{\"id\":\"{id}\",\"launcher_version\":\"{}\"}}",
            env!("CARGO_PKG_VERSION")
        );
        fsutil::atomic_write(&marker, body.as_bytes()).map_err(LauncherError::io(&marker))
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))?
}

/// Paths relative to the data root, `/`-separated (for repair reports).
pub(crate) fn relative(root: &Path, paths: &[std::path::PathBuf]) -> Vec<String> {
    paths
        .iter()
        .take(200)
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}
