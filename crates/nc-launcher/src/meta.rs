//! Fetching and caching of official Mojang metadata.

use nc_core::manifest::{
    AssetIndex, AssetIndexRef, ManifestEntry, VERSION_MANIFEST_URL, VersionJson, VersionManifest,
};
use tokio_util::sync::CancellationToken;

use crate::{
    LauncherError,
    download::{DownloadTask, Downloader},
    fsutil,
    layout::Layout,
};

#[derive(Clone)]
pub struct Meta {
    pub dl: Downloader,
    pub layout: Layout,
    pub manifest_url: String,
}

impl Meta {
    pub fn new(dl: Downloader, layout: Layout) -> Self {
        Self {
            dl,
            layout,
            manifest_url: VERSION_MANIFEST_URL.into(),
        }
    }

    /// Latest manifest from the network, falling back to the last cached copy
    /// when offline. Returns `(manifest, from_cache)`.
    pub async fn version_manifest(
        &self,
        cancel: &CancellationToken,
    ) -> Result<(VersionManifest, bool), LauncherError> {
        let cache = self.layout.cache_dir().join("version_manifest_v2.json");
        match self.dl.get_bytes(&self.manifest_url, cancel).await {
            Ok(bytes) => {
                let m: VersionManifest = serde_json::from_slice(&bytes)
                    .map_err(LauncherError::parse("version manifest"))?;
                let c = cache.clone();
                // Cache write failures are not fatal.
                let _ = tokio::task::spawn_blocking(move || fsutil::atomic_write(&c, &bytes)).await;
                Ok((m, false))
            }
            Err(crate::download::DownloadError::Cancelled) => Err(LauncherError::Cancelled),
            Err(e) => {
                tracing::warn!(error = %e, "manifest fetch failed, trying cache");
                let bytes = tokio::fs::read(&cache)
                    .await
                    .map_err(|_| LauncherError::Download(e))?;
                let m = serde_json::from_slice(&bytes)
                    .map_err(LauncherError::parse("cached version manifest"))?;
                Ok((m, true))
            }
        }
    }

    /// Version JSON, downloaded once and verified against the manifest SHA-1.
    pub async fn version_json(
        &self,
        entry: &ManifestEntry,
        cancel: &CancellationToken,
    ) -> Result<VersionJson, LauncherError> {
        let dest = self.layout.version_json(&entry.id)?;
        self.dl
            .fetch_all(
                vec![DownloadTask {
                    url: entry.url.clone(),
                    dest: dest.clone(),
                    sha1: Some(entry.sha1.clone()),
                    size: None,
                }],
                cancel,
                |_| {},
            )
            .await?;
        read_json(&dest, &format!("version {}", entry.id)).await
    }

    pub async fn asset_index(
        &self,
        r: &AssetIndexRef,
        cancel: &CancellationToken,
    ) -> Result<AssetIndex, LauncherError> {
        let dest = self.layout.asset_index(&r.id)?;
        self.dl
            .fetch_all(
                vec![DownloadTask {
                    url: r.url.clone(),
                    dest: dest.clone(),
                    sha1: Some(r.sha1.clone()),
                    size: Some(r.size),
                }],
                cancel,
                |_| {},
            )
            .await?;
        read_json(&dest, &format!("asset index {}", r.id)).await
    }
}

pub async fn read_json<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    what: &str,
) -> Result<T, LauncherError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(LauncherError::io(path))?;
    serde_json::from_slice(&bytes).map_err(LauncherError::parse(what))
}
