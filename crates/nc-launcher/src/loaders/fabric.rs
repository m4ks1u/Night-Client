//! Fabric and Quilt: a version JSON from Fabric Meta or Quilt Meta that
//! inherits from vanilla. Both services publish the same profile format.
//!
//! The profiles omit library hashes, so each SHA-1 is taken from the Maven
//! `.sha1` sidecar next to each jar (same HTTPS origin) before download.

use nc_core::{libraries, loader::InstallKey, manifest::VersionJson, rules::Environment};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::LoaderVersion;
/// Which Fabric-style meta service a profile comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Fabric,
    Quilt,
}

impl Flavor {
    fn name(self) -> &'static str {
        match self {
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
        }
    }

    /// `<meta base>/v<n>/versions/loader`
    fn loader_url(self, meta: &Meta) -> String {
        match self {
            Self::Fabric => format!("{}/v2/versions/loader", meta.fabric_meta_url),
            Self::Quilt => format!("{}/v3/versions/loader", meta.quilt_meta_url),
        }
    }
}

use crate::{
    LauncherError,
    download::{DownloadTask, Progress},
    fsutil,
    install::{self, InstallProgress, Installed, Phase, ProgressFn},
    meta::Meta,
};

#[derive(Deserialize)]
struct Entry {
    loader: LoaderInfo,
}

#[derive(Deserialize)]
struct LoaderInfo {
    version: String,
}

pub async fn versions(
    flavor: Flavor,
    meta: &Meta,
    minecraft: &str,
    cancel: &CancellationToken,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    nc_core::paths::validate_segment(minecraft)?;
    let url = format!("{}/{minecraft}", flavor.loader_url(meta));
    let bytes = match meta.dl.get_bytes(&url, cancel).await {
        Ok(b) => b,
        // Fabric Meta answers 400, Quilt Meta 404, for unsupported versions.
        Err(crate::download::DownloadError::Http {
            status: 400 | 404, ..
        }) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let list: Vec<Entry> = serde_json::from_slice(&bytes).map_err(LauncherError::parse(
        format!("{} loader list", flavor.name()),
    ))?;
    // Fabric Meta flags only the newest loader as `stable`; older releases are
    // still releases. Pre-releases carry a SemVer suffix (0.17.0-beta.1).
    // Quilt Meta returns its list unordered, so both are sorted newest first.
    let mut out: Vec<LoaderVersion> = list
        .into_iter()
        .map(|e| LoaderVersion {
            stable: !e.loader.version.contains('-'),
            version: e.loader.version,
        })
        .collect();
    out.sort_by(|a, b| nc_core::loader::version_cmp(&b.version, &a.version));
    Ok(out)
}

fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub async fn install(
    flavor: Flavor,
    meta: &Meta,
    key: &InstallKey,
    env: &Environment,
    cancel: &CancellationToken,
    on_progress: ProgressFn,
) -> Result<Installed, LauncherError> {
    let lv = key
        .loader_version
        .as_deref()
        .ok_or(nc_core::CoreError::MissingField {
            version: key.to_key(),
            field: "loader version",
        })?;
    let id = key.launch_id();
    let layout = &meta.layout;

    let vanilla =
        install::install_vanilla(meta, &key.minecraft, env, cancel, on_progress.clone()).await?;
    let emit = {
        let f = on_progress.clone();
        move |p: Progress| {
            f(InstallProgress {
                phase: Phase::Loader,
                progress: p,
            })
        }
    };
    emit(Progress::default());
    let was_installed = super::begin(layout, &id).await?;

    let url = format!(
        "{}/{}/{lv}/profile/json",
        flavor.loader_url(meta),
        key.minecraft
    );
    let json = meta.dl.get_bytes(&url, cancel).await?;
    let child: VersionJson = serde_json::from_slice(&json)
        .map_err(LauncherError::parse(format!("{} profile", flavor.name())))?;
    if child.id != id || child.inherits_from.as_deref() != Some(key.minecraft.as_str()) {
        return Err(LauncherError::Parse {
            what: format!("{} profile", flavor.name()),
            message: format!(
                "unexpected id {} / parent {:?}",
                child.id, child.inherits_from
            ),
        });
    }

    let mut tasks = Vec::new();
    for lib in libraries::resolve(&child, env)? {
        let Some(url) = lib.url.clone() else {
            return Err(nc_core::CoreError::MissingField {
                version: id.clone(),
                field: "library url",
            }
            .into());
        };
        let sha1 = match lib.sha1.clone() {
            Some(h) => h,
            None => {
                let text = meta.dl.get_bytes(&format!("{url}.sha1"), cancel).await?;
                let h = String::from_utf8_lossy(&text)
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !is_sha1(&h) {
                    return Err(LauncherError::Parse {
                        what: format!("{url}.sha1"),
                        message: "not a SHA-1".into(),
                    });
                }
                h
            }
        };
        tasks.push(DownloadTask {
            url,
            dest: fsutil::safe_join(&layout.libraries_dir(), &lib.path)?,
            sha1: Some(sha1),
            size: lib.size,
        });
    }
    let files = meta.dl.fetch_all(tasks, cancel, emit).await?;

    super::finish(layout, &id, &json, &vanilla.client_jar).await?;
    let merged = super::resolve_version(layout, &id).await?;
    let libs = libraries::resolve(&merged, env)?;
    let repaired_count = vanilla.repaired_count
        + if was_installed {
            files.downloaded_files
        } else {
            0
        };
    let mut repaired = vanilla.repaired.clone();
    if was_installed {
        repaired.extend(super::relative(layout.root(), &files.fetched));
    }
    tracing::info!(%id, loader = flavor.name(), libraries = files.downloaded_files, "loader installed");
    Ok(Installed {
        version: merged,
        libraries: libs,
        java: vanilla.java,
        client_jar: layout.client_jar(&id)?,
        log_config: vanilla.log_config,
        files: crate::download::Summary {
            downloaded_files: vanilla.files.downloaded_files + files.downloaded_files,
            skipped_files: vanilla.files.skipped_files + files.skipped_files,
            downloaded_bytes: vanilla.files.downloaded_bytes + files.downloaded_bytes,
            fetched: Vec::new(),
        },
        was_installed: was_installed || vanilla.was_installed,
        repaired,
        repaired_count,
    })
}
