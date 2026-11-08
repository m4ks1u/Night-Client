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
        modpack::{self, Tracked},
        packupdate::{self, UpdateReport},
    },
};
use nc_store::{InstancePatch, ModpackRecord, PackFileRecord};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use super::{
    CmdError, Result,
    instances::{self, InstanceView},
    invalid, validate_instance_id, with_db,
};
use crate::state::{AppState, lock};

/// A pack waiting to be imported.
pub struct PendingPack {
    path: PathBuf,
    pack: Pack,
    origin: Origin,
    cancel: CancellationToken,
    importing: bool,
}

/// Where a pack came from on Modrinth (unknown for a file Modrinth doesn't list).
#[derive(Clone, Default)]
struct Origin {
    project_id: Option<String>,
    version_id: Option<String>,
    icon_url: Option<String>,
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
fn remember(state: &AppState, path: PathBuf, pack: Pack, origin: Origin) -> Result<PackPreview> {
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
        icon_url: origin.icon_url.clone(),
    };
    let mut pending = lock(&state.modpacks);
    pending.retain(|_, p| p.importing);
    pending.insert(
        token,
        PendingPack {
            path,
            pack,
            origin,
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
    let origin = identify(&state, &path).await;
    remember(&state, path, pack, origin).map(Some)
}

/// Look a pack file up on Modrinth by SHA-512, so a pack imported from a
/// file can still be updated. Best effort: offline or unknown means none.
async fn identify(state: &AppState, path: &std::path::Path) -> Origin {
    let p = path.to_owned();
    let Ok(Ok(sha512)) = tauri::async_runtime::spawn_blocking(move || modpack::sha512_of(&p)).await
    else {
        return Origin::default();
    };
    let c = CancellationToken::new();
    let Ok(found) = state
        .modrinth
        .versions_by_hash(std::slice::from_ref(&sha512), &c)
        .await
    else {
        return Origin::default();
    };
    let Some(v) = found.into_values().next() else {
        return Origin::default();
    };
    match state.modrinth.project(&v.project_id, &c).await {
        Ok(p) if p.project_type == "modpack" => Origin {
            project_id: Some(p.id),
            version_id: Some(v.id),
            icon_url: p.icon_url,
        },
        _ => Origin::default(),
    }
}

/// Download a Modrinth modpack (the newest installable release, or
/// `version_id`) and preview it.
#[tauri::command]
pub async fn modpack_preview_modrinth(
    state: State<'_, AppState>,
    project_id: String,
    version_id: Option<String>,
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
    let versions = match version_id {
        // A specific version (it must belong to this project).
        Some(vid) => {
            if !valid_id(&vid) {
                return Err(invalid("invalid version"));
            }
            let v = state
                .modrinth
                .version(&vid, &c)
                .await
                .map_err(LauncherError::from)?;
            if v.project_id != project.id {
                return Err(invalid("that version belongs to another project"));
            }
            vec![v]
        }
        None => state
            .modrinth
            .all_versions(&project.id, &c)
            .await
            .map_err(LauncherError::from)?,
    };
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
    let origin = Origin {
        project_id: Some(project.id),
        version_id: Some(v.id.clone()),
        icon_url: project.icon_url,
    };
    remember(&state, path, pack, origin)
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
    let (path, pack, origin, cancel) = {
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
        (
            p.path.clone(),
            p.pack.clone(),
            p.origin.clone(),
            p.cancel.clone(),
        )
    };
    let result = import(&app, &state, &token, &path, &pack, &origin, name, &cancel).await;
    lock(&state.modpacks).remove(&token);
    result
}

#[allow(clippy::too_many_arguments)]
async fn import(
    app: &AppHandle,
    state: &AppState,
    token: &str,
    path: &std::path::Path,
    pack: &Pack,
    origin: &Origin,
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
        modpack::download_files(
            &state.meta().dl,
            &game_dir,
            &pack.files,
            cancel,
            emitter(app, token, Some((total_files, total_bytes))),
        )
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
        let tracked = modpack::tracked_after_install(pack, &n);
        let (rid, rec, files) = (id.clone(), record(pack, origin), to_rows(&tracked));
        with_db(state, move |db| db.set_modpack(&rid, &rec, &files)).await?;
        Ok::<usize, CmdError>(n.len())
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

/// Forward download progress as `modpack://progress`, at most every 150 ms.
/// `totals` replaces the engine's (an import knows the whole pack's size).
fn emitter(
    app: &AppHandle,
    token: &str,
    totals: Option<(u64, u64)>,
) -> impl Fn(nc_launcher::download::Progress) + Send + Sync + 'static {
    let (app, token) = (app.clone(), token.to_owned());
    let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
    move |p| {
        let mut l = last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let finished = p.done_files + p.skipped_files >= p.total_files;
        if l.elapsed() < Duration::from_millis(150) && !finished {
            return;
        }
        *l = Instant::now();
        let (total_files, total_bytes) = totals.unwrap_or((p.total_files, p.total_bytes));
        let _ = app.emit(
            "modpack://progress",
            ImportProgress {
                token: &token,
                phase: "files",
                done_files: p.done_files + p.skipped_files,
                total_files,
                done_bytes: p.done_bytes,
                total_bytes,
            },
        );
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn record(pack: &Pack, origin: &Origin) -> ModpackRecord {
    ModpackRecord {
        project_id: origin.project_id.clone(),
        version_id: origin.version_id.clone(),
        name: pack.name.clone(),
        version_number: pack.version_id.clone(),
        icon_url: origin.icon_url.clone(),
        installed_at: now(),
    }
}

fn to_rows(tracked: &[Tracked]) -> Vec<PackFileRecord> {
    tracked
        .iter()
        .map(|t| PackFileRecord {
            path: t.path.clone(),
            sha1: t.sha1.clone(),
            is_override: t.is_override,
        })
        .collect()
}

fn from_rows(rows: Vec<PackFileRecord>) -> Vec<Tracked> {
    rows.into_iter()
        .map(|r| Tracked {
            path: r.path,
            sha1: r.sha1,
            is_override: r.is_override,
        })
        .collect()
}

/// Cancel an import or an update in progress. A cancelled import discards
/// the half-made instance; a cancelled update changes nothing.
#[tauri::command]
pub fn modpack_cancel(state: State<'_, AppState>, token: String) {
    if let Some(p) = lock(&state.modpacks).get(&token) {
        p.cancel.cancel();
    }
    if let Some(c) = lock(&state.modpack_updates).get(&token) {
        c.cancel();
    }
}

/// The modpack an instance was made from, if any.
#[tauri::command]
pub async fn modpack_info(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Option<ModpackRecord>> {
    validate_instance_id(&instance_id)?;
    with_db(&state, move |db| db.get_modpack(&instance_id)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestPack {
    version_id: String,
    version_number: String,
    name: String,
    date_published: String,
    game_versions: Vec<String>,
}

/// The newest installable version of the instance's modpack, when it isn't
/// the installed one. `None`: up to date.
#[tauri::command]
pub async fn modpack_check_update(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Option<LatestPack>> {
    validate_instance_id(&instance_id)?;
    let rec = with_db(&state, move |db| db.get_modpack(&instance_id))
        .await?
        .ok_or_else(|| CmdError::new("not_found", "This instance wasn't made from a modpack."))?;
    let Some(project) = rec.project_id else {
        return Err(CmdError::new(
            "unknown_pack",
            "This modpack isn't on Modrinth, so it can't be updated from here.",
        ));
    };
    let versions = state
        .modrinth
        .all_versions(&project, &CancellationToken::new())
        .await
        .map_err(LauncherError::from)?;
    Ok(pick_modpack_version(&versions)
        .filter(|v| Some(&v.id) != rec.version_id.as_ref())
        .map(|v| LatestPack {
            version_id: v.id.clone(),
            version_number: v.version_number.clone(),
            name: v.name.clone(),
            date_published: v.date_published.clone(),
            game_versions: v.game_versions.clone(),
        }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Updated {
    report: UpdateReport,
    instance: InstanceView,
}

/// Update an instance to another version of its modpack. Files the user
/// changed are kept; nothing changes unless every new file was verified.
#[tauri::command]
pub async fn modpack_update(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    version_id: String,
) -> Result<Updated> {
    validate_instance_id(&instance_id)?;
    if !valid_id(&version_id) {
        return Err(invalid("invalid version"));
    }
    super::ensure_not_relocating(&state)?;
    if lock(&state.games).contains_key(&instance_id) {
        return Err(CmdError::new(
            "busy",
            "Close the game before updating its modpack.",
        ));
    }
    let cancel = CancellationToken::new();
    {
        let mut running = lock(&state.modpack_updates);
        if running.contains_key(&instance_id) {
            return Err(CmdError::new(
                "busy",
                "This modpack is already being updated.",
            ));
        }
        running.insert(instance_id.clone(), cancel.clone());
    }
    let result = update(&app, &state, &instance_id, &version_id, &cancel).await;
    lock(&state.modpack_updates).remove(&instance_id);
    if cancel.is_cancelled() && result.is_err() {
        return Err(CmdError::new(
            "cancelled",
            "Update cancelled. Nothing was changed.",
        ));
    }
    result
}

async fn update(
    app: &AppHandle,
    state: &AppState,
    instance_id: &str,
    version_id: &str,
    cancel: &CancellationToken,
) -> Result<Updated> {
    let id = instance_id.to_owned();
    let (inst, rec, rows) = with_db(state, move |db| {
        Ok((
            db.get_instance(&id)?,
            db.get_modpack(&id)?,
            db.modpack_files(&id)?,
        ))
    })
    .await?;
    let rec =
        rec.ok_or_else(|| CmdError::new("not_found", "This instance wasn't made from a modpack."))?;
    let Some(project) = rec.project_id.clone() else {
        return Err(CmdError::new(
            "unknown_pack",
            "This modpack isn't on Modrinth, so it can't be updated from here.",
        ));
    };
    let v = state
        .modrinth
        .version(version_id, cancel)
        .await
        .map_err(LauncherError::from)?;
    if v.project_id != project {
        return Err(invalid("that version belongs to another project"));
    }
    let v = pick_modpack_version(std::slice::from_ref(&v))
        .ok_or_else(|| CmdError::new("unsupported_pack", "That version can't be installed."))?;
    let file = v.primary_file().expect("picked versions have a file");
    let cache = state.layout.cache_dir().join("modpacks");
    let path = modpack::fetch_pack(&state.meta().dl, &cache, file, cancel).await?;
    let pack = read(path.clone()).await?;
    let key = key_of(&pack)?;
    let current = instances::install_key(&inst)
        .ok_or_else(|| CmdError::new("invalid_input", "This instance's loader is invalid."))?;
    if key.kind != current.kind {
        return Err(CmdError::new(
            "unsupported_pack",
            format!(
                "This version uses another loader ({}); create a new instance from it instead.",
                key.label()
            ),
        ));
    }
    let game_dir = match &inst.game_dir {
        Some(d) => PathBuf::from(d),
        None => nc_launcher::instance::ensure_game_dir(&state.layout, &inst.dir_name)?,
    };
    let report = packupdate::update(
        &state.meta().dl,
        &game_dir,
        &path,
        &pack,
        &from_rows(rows),
        cancel,
        emitter(app, instance_id, None),
    )
    .await?;
    // From here on the files are in place: record them before anything else.
    let origin = Origin {
        project_id: Some(project),
        version_id: Some(v.id.clone()),
        icon_url: rec.icon_url.clone(),
    };
    let (id, new_rec, files) = (
        inst.id.clone(),
        record(&pack, &origin),
        to_rows(&report.tracked),
    );
    let patch = InstancePatch {
        game_version: (key.minecraft != inst.game_version).then(|| key.minecraft.clone()),
        loader_version: key
            .loader_version
            .clone()
            .filter(|lv| Some(lv) != inst.loader_version.as_ref()),
        ..Default::default()
    };
    let updated = with_db(state, move |db| {
        db.set_modpack(&id, &new_rec, &files)?;
        db.update_instance(&id, patch)
    })
    .await?;
    tracing::info!(
        id = %updated.id,
        version = %pack.version_id,
        added = report.added,
        replaced = report.replaced,
        removed = report.removed,
        kept = report.kept.len(),
        "modpack updated"
    );
    Ok(Updated {
        report,
        instance: instances::view(state, updated),
    })
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
