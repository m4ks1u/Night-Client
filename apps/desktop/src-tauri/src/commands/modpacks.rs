//! Modrinth modpacks (`.mrpack`): search, preview, import as a new instance.
//!
//! A preview reads and validates the pack and keeps it here behind a
//! token; the WebView never supplies a file path. Import creates the
//! instance, downloads every file (HTTPS from allowed hosts, size, SHA-1 and
//! SHA-512 checked) and copies the overrides. If anything fails, the new
//! instance is discarded.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use nc_core::{
    loader::{InstallKey, LoaderKind},
    mrpack::Pack,
};
use nc_launcher::{
    CancellationToken, LauncherError,
    modrinth::{
        api::{SortIndex, pick_modpack_version, valid_id},
        modpack,
    },
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use super::{
    CmdError, Result,
    instances::{self, InstanceView},
    invalid,
};
use crate::state::{AppState, lock};

/// A pack waiting to be imported.
pub struct PendingPack {
    path: PathBuf,
    pack: Pack,
    cancel: CancellationToken,
    importing: bool,
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackPreview {
    token: String,
    name: String,
    version_id: String,
    summary: Option<String>,
    minecraft: String,
    loader: LoaderKind,
    loader_version: Option<String>,
    /// e.g. "Fabric 0.16.10 · 1.21.1".
    label: String,
    files: usize,
    optional_files: usize,
    /// Server-only files the pack lists; not downloaded.
    skipped: usize,
    download_bytes: u64,
    icon_url: Option<String>,
}

fn key_of(pack: &Pack) -> Result<InstallKey> {
    InstallKey::new(pack.loader, &pack.minecraft, pack.loader_version.as_deref()).map_err(|_| {
        CmdError::new(
            "unsupported_pack",
            format!(
                "Night Client can't install this pack's loader ({:?} {} for {}).",
                pack.loader,
                pack.loader_version.as_deref().unwrap_or("?"),
                pack.minecraft
            ),
        )
    })
}

/// Keep the pack behind a new token. Previews nobody imported are dropped.
fn remember(
    state: &AppState,
    path: PathBuf,
    pack: Pack,
    icon_url: Option<String>,
) -> Result<PackPreview> {
    let key = key_of(&pack)?;
    let token = format!("pack-{}", NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let preview = PackPreview {
        token: token.clone(),
        name: pack.name.clone(),
        version_id: pack.version_id.clone(),
        summary: pack.summary.clone(),
        minecraft: pack.minecraft.clone(),
        loader: pack.loader,
        loader_version: pack.loader_version.clone(),
        label: key.label(),
        files: pack.files.len(),
        optional_files: pack.files.iter().filter(|f| f.optional).count(),
        skipped: pack.skipped,
        download_bytes: pack.download_bytes(),
        icon_url,
    };
    let mut pending = lock(&state.modpacks);
    pending.retain(|_, p| p.importing);
    pending.insert(
        token,
        PendingPack {
            path,
            pack,
            cancel: CancellationToken::new(),
            importing: false,
        },
    );
    Ok(preview)
}

async fn read(path: PathBuf) -> Result<Pack> {
    let p = path.clone();
    tauri::async_runtime::spawn_blocking(move || modpack::read_pack(&p))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
        .map_err(|e| CmdError::new("invalid_pack", format!("This isn't a valid modpack: {e}")))
}

/// Pick a `.mrpack` with the native dialog and preview it. `None` when the
/// dialog was cancelled.
#[tauri::command]
pub async fn modpack_preview_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<PackPreview>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import a modpack")
        .add_filter("Modrinth modpack", &["mrpack"])
        .pick_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let Some(path) = rx
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
    else {
        return Ok(None);
    };
    let pack = read(path.clone()).await?;
    remember(&state, path, pack, None).map(Some)
}

/// Download the newest release of a Modrinth modpack project and preview it.
#[tauri::command]
pub async fn modpack_preview_modrinth(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<PackPreview> {
    if !valid_id(&project_id) {
        return Err(invalid("invalid project"));
    }
    let c = CancellationToken::new();
    let project = state
        .modrinth
        .project(&project_id, &c)
        .await
        .map_err(LauncherError::from)?;
    if project.project_type != "modpack" {
        return Err(invalid("that project isn't a modpack"));
    }
    let versions = state
        .modrinth
        .all_versions(&project.id, &c)
        .await
        .map_err(LauncherError::from)?;
    let v = pick_modpack_version(&versions).ok_or_else(|| {
        CmdError::new(
            "unsupported_pack",
            "This modpack has no version Night Client can install (Fabric, Quilt, NeoForge or Forge).",
        )
    })?;
    let file = v.primary_file().expect("picked versions have a file");
    let cache = state.layout.cache_dir().join("modpacks");
    let path = modpack::fetch_pack(&state.meta().dl, &cache, file, &c).await?;
    let pack = read(path.clone()).await?;
    remember(&state, path, pack, project.icon_url)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ImportProgress<'a> {
    token: &'a str,
    /// "files" or "overrides".
    phase: &'static str,
    done_files: u64,
    total_files: u64,
    done_bytes: u64,
    total_bytes: u64,
}

/// Import a previewed pack as a new instance named `name`.
#[tauri::command]
pub async fn modpack_import(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    name: String,
) -> Result<InstanceView> {
    super::ensure_not_relocating(&state)?;
    let (path, pack, cancel) = {
        let mut pending = lock(&state.modpacks);
        let p = pending
            .get_mut(&token)
            .ok_or_else(|| CmdError::new("not_found", "Preview the modpack again."))?;
        if p.importing {
            return Err(CmdError::new(
                "busy",
                "This modpack is already being imported.",
            ));
        }
        p.importing = true;
        (p.path.clone(), p.pack.clone(), p.cancel.clone())
    };
    let result = import(&app, &state, &token, &path, &pack, name, &cancel).await;
    lock(&state.modpacks).remove(&token);
    result
}

async fn import(
    app: &AppHandle,
    state: &AppState,
    token: &str,
    path: &std::path::Path,
    pack: &Pack,
    name: String,
    cancel: &CancellationToken,
) -> Result<InstanceView> {
    let key = key_of(pack)?;
    let created = instances::create(state, name, &key, None).await?;
    let game_dir = nc_launcher::instance::ensure_game_dir(&state.layout, &created.dir_name)?;
    let id = created.id.clone();
    let work = async {
        let total_files = pack.files.len() as u64;
        let total_bytes = pack.download_bytes();
        let (emitter, tok) = (app.clone(), token.to_owned());
        let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
        modpack::download_files(&state.meta().dl, &game_dir, &pack.files, cancel, move |p| {
            let mut l = last
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let finished = p.done_files + p.skipped_files >= p.total_files;
            if l.elapsed() < Duration::from_millis(150) && !finished {
                return;
            }
            *l = Instant::now();
            let _ = emitter.emit(
                "modpack://progress",
                ImportProgress {
                    token: &tok,
                    phase: "files",
                    done_files: p.done_files + p.skipped_files,
                    total_files,
                    done_bytes: p.done_bytes,
                    total_bytes,
                },
            );
        })
        .await?;
        let _ = app.emit(
            "modpack://progress",
            ImportProgress {
                token,
                phase: "overrides",
                done_files: total_files,
                total_files,
                done_bytes: total_bytes,
                total_bytes,
            },
        );
        let (src, dst) = (path.to_owned(), game_dir.clone());
        let n = tauri::async_runtime::spawn_blocking(move || {
            modpack::extract_overrides(&src, &dst, modpack::MAX_OVERRIDES_BYTES)
        })
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
        Ok::<usize, CmdError>(n)
    };
    match work.await {
        Ok(overrides) => {
            tracing::info!(id = %id, files = pack.files.len(), overrides, "modpack imported");
            Ok(instances::view(state, created))
        }
        Err(e) => {
            tracing::warn!(id = %id, error = %e.message, "modpack import failed; discarding the instance");
            instances::discard(state, id).await;
            if cancel.is_cancelled() {
                Err(CmdError::new("cancelled", "Import cancelled."))
            } else {
                Err(e)
            }
        }
    }
}

/// Cancel an import in progress (the half-made instance is discarded).
#[tauri::command]
pub fn modpack_cancel(state: State<'_, AppState>, token: String) {
    if let Some(p) = lock(&state.modpacks).get(&token) {
        p.cancel.cancel();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackHits {
    hits: Vec<nc_launcher::modrinth::api::SearchHit>,
    total: u32,
    offset: u32,
}

#[tauri::command]
pub async fn modpack_search(
    state: State<'_, AppState>,
    query: String,
    sort: SortIndex,
    offset: u32,
) -> Result<PackHits> {
    if query.len() > 200 {
        return Err(invalid("search text is too long"));
    }
    let r = state
        .modrinth
        .search_modpacks(&query, sort, offset, 20, &CancellationToken::new())
        .await
        .map_err(LauncherError::from)?;
    Ok(PackHits {
        hits: r.hits,
        total: r.total_hits,
        offset: r.offset,
    })
}

pub type Pending = HashMap<String, PendingPack>;
