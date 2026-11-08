//! Modrinth discovery and per-instance mod management.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use nc_launcher::{
    CancellationToken, LauncherError,
    modrinth::{
        api::{Category, Project, SearchHit, SearchQuery, SortIndex, Version, valid_id},
        mods::{self, ModFile},
        plan::{self, Plan, pick_best},
    },
};
use nc_store::{Instance, Loader, ModRecord, ModSource};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::{CmdError, Result, invalid, validate_instance_id, with_db};
use crate::state::{AppState, lock};

/// Instance + its loader name + mods folder; refuses vanilla instances.
async fn ctx(state: &AppState, id: &str) -> Result<(Instance, &'static str, PathBuf)> {
    validate_instance_id(id)?;
    let lookup = id.to_owned();
    let inst = with_db(state, move |db| db.get_instance(&lookup)).await?;
    let loader = match inst.loader {
        Loader::Fabric => "fabric",
        Loader::Neoforge => "neoforge",
        Loader::Vanilla => {
            return Err(CmdError::new(
                "no_loader",
                "Vanilla instances can't use mods. Create a Fabric or NeoForge instance.",
            ));
        }
    };
    let game_dir = match &inst.game_dir {
        Some(d) => PathBuf::from(d),
        None => state
            .layout
            .instance_dir(&inst.dir_name)
            .map_err(LauncherError::from)?,
    };
    Ok((inst, loader, mods::mods_dir(&game_dir)))
}

fn ensure_not_running(state: &AppState, id: &str) -> Result<()> {
    super::ensure_not_relocating(state)?;
    if lock(&state.games).contains_key(id) {
        Err(CmdError::new(
            "busy",
            "Close the game before changing its mods.",
        ))
    } else {
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitView {
    #[serde(flatten)]
    hit: SearchHit,
    installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchView {
    hits: Vec<HitView>,
    total: u32,
    offset: u32,
}

#[tauri::command]
pub async fn modrinth_search(
    state: State<'_, AppState>,
    instance_id: String,
    query: String,
    category: Option<String>,
    sort: SortIndex,
    offset: u32,
) -> Result<SearchView> {
    if query.len() > 200 {
        return Err(invalid("search text is too long"));
    }
    if category.as_deref().is_some_and(|c| !valid_id(c)) {
        return Err(invalid("invalid category"));
    }
    let (inst, loader, _) = ctx(&state, &instance_id).await?;
    let id = inst.id.clone();
    let installed: HashSet<String> = with_db(&state, move |db| db.list_mods(&id))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let r = state
        .modrinth
        .search(
            &SearchQuery {
                query: query.trim(),
                loader,
                minecraft: &inst.game_version,
                category: category.as_deref(),
                sort,
                offset,
                limit: 20,
            },
            &CancellationToken::new(),
        )
        .await
        .map_err(LauncherError::from)?;
    Ok(SearchView {
        total: r.total_hits,
        offset: r.offset,
        hits: r
            .hits
            .into_iter()
            .map(|h| HitView {
                installed: installed.contains(&h.project_id),
                hit: h,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn modrinth_project(state: State<'_, AppState>, project_id: String) -> Result<Project> {
    Ok(state
        .modrinth
        .project(&project_id, &CancellationToken::new())
        .await
        .map_err(LauncherError::from)?)
}

/// Open a project's Modrinth page in the system browser. Only validated
/// slugs on modrinth.com can be opened this way.
#[tauri::command]
pub fn open_modrinth_page(app: AppHandle, slug: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    if !valid_id(&slug) {
        return Err(invalid("invalid project"));
    }
    app.opener()
        .open_url(format!("https://modrinth.com/mod/{slug}"), None::<&str>)
        .map_err(|e| CmdError::new("io", e.to_string()))
}

#[tauri::command]
pub async fn modrinth_categories(state: State<'_, AppState>) -> Result<Vec<Category>> {
    Ok(state
        .modrinth
        .categories(&CancellationToken::new())
        .await
        .map_err(LauncherError::from)?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    #[serde(flatten)]
    plan: Plan,
    /// Project titles for every id mentioned in the plan.
    titles: HashMap<String, String>,
}

/// What installing a project would do: the chosen version, required
/// dependencies, already-satisfied ones, missing ones and declared conflicts.
#[tauri::command]
pub async fn mod_install_plan(
    state: State<'_, AppState>,
    instance_id: String,
    project_id: String,
) -> Result<PlanView> {
    if !valid_id(&project_id) {
        return Err(invalid("invalid project"));
    }
    let (inst, loader, _) = ctx(&state, &instance_id).await?;
    let c = CancellationToken::new();
    let mr = &state.modrinth;
    let versions = mr
        .project_versions(&project_id, loader, &inst.game_version, &c)
        .await
        .map_err(LauncherError::from)?;
    let root = pick_best(versions, loader, &inst.game_version).ok_or(CmdError::new(
        "mod_incompatible",
        format!(
            "This mod has no version for {loader} on Minecraft {}.",
            inst.game_version
        ),
    ))?;
    let id = inst.id.clone();
    let installed: HashSet<String> = with_db(&state, move |db| db.list_mods(&id))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let p = plan::plan(mr, root, loader, &inst.game_version, &installed, &c)
        .await
        .map_err(LauncherError::from)?;

    let mut titles = HashMap::new();
    let ids: Vec<String> = p
        .install
        .iter()
        .map(|m| m.version.project_id.clone())
        .chain(p.missing.iter().cloned())
        .chain(p.conflicts.iter().cloned())
        .chain(p.already_installed.iter().cloned())
        .collect();
    for pid in ids {
        if let Ok(proj) = mr.project(&pid, &c).await {
            titles.insert(pid, proj.title);
        }
    }
    Ok(PlanView { plan: p, titles })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallItem {
    version_id: String,
    required_by: Option<String>,
}

/// Install the versions of a reviewed plan. Each is re-fetched and
/// re-checked for loader/game-version compatibility before download.
#[tauri::command]
pub async fn mod_install(
    state: State<'_, AppState>,
    instance_id: String,
    items: Vec<InstallItem>,
) -> Result<Vec<ModRecord>> {
    if items.is_empty() || items.len() > 64 {
        return Err(invalid("nothing to install"));
    }
    let (inst, loader, dir) = ctx(&state, &instance_id).await?;
    ensure_not_running(&state, &inst.id)?;
    let c = CancellationToken::new();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(LauncherError::io(&dir))?;
    let mut done = Vec::new();
    for item in items {
        if !valid_id(&item.version_id) || item.required_by.as_deref().is_some_and(|r| !valid_id(r))
        {
            return Err(invalid("invalid version"));
        }
        let v: Version = state
            .modrinth
            .version(&item.version_id, &c)
            .await
            .map_err(LauncherError::from)?;
        if !v.supports(loader, &inst.game_version) {
            return Err(CmdError::new(
                "mod_incompatible",
                format!("{} is not for {loader} {}.", v.name, inst.game_version),
            ));
        }
        done.push(install_and_record(&state, &inst.id, &dir, &v, item.required_by, &c).await?);
    }
    Ok(done)
}

/// Download and verify one Modrinth version into the mods folder and store
/// its row (title and icon from the project, when reachable).
async fn install_and_record(
    state: &AppState,
    instance_id: &str,
    dir: &std::path::Path,
    v: &Version,
    required_by: Option<String>,
    c: &CancellationToken,
) -> Result<ModRecord> {
    let file = mods::install_version(&state.meta().dl, dir, v, c).await?;
    let proj = state.modrinth.project(&v.project_id, c).await.ok();
    let rec = ModRecord {
        instance_id: instance_id.to_owned(),
        file_name: file.file_name.clone(),
        source: ModSource::Modrinth,
        project_id: Some(v.project_id.clone()),
        version_id: Some(v.id.clone()),
        title: proj
            .as_ref()
            .map(|p| p.title.clone())
            .unwrap_or_else(|| v.name.clone()),
        version_number: Some(v.version_number.clone()),
        icon_url: proj.and_then(|p| p.icon_url),
        sha1: file.sha1.clone(),
        sha512: file.sha512.clone(),
        size: file.size as i64,
        enabled: true,
        required_by,
        installed_at: 0,
    };
    let r = rec.clone();
    with_db(state, move |db| db.upsert_mod(&r)).await?;
    tracing::info!(instance = %instance_id, file = %file.file_name, "mod installed");
    Ok(rec)
}

fn stem(file_name: &str) -> String {
    file_name
        .strip_suffix(".jar")
        .unwrap_or(file_name)
        .to_owned()
}

/// List mods, re-syncing metadata with the folder: rows for deleted files
/// are dropped, new or replaced files are identified by SHA-512 on Modrinth
/// (offline: recorded as local), and enabled state follows the disk.
#[tauri::command]
pub async fn list_mods(state: State<'_, AppState>, instance_id: String) -> Result<Vec<ModRecord>> {
    let (inst, _, dir) = ctx(&state, &instance_id).await?;
    let scan_dir = dir.clone();
    let files: Vec<ModFile> = tauri::async_runtime::spawn_blocking(move || mods::scan(&scan_dir))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
    let id = inst.id.clone();
    let rows = with_db(&state, move |db| db.list_mods(&id)).await?;
    let by_name: HashMap<&str, &ModRecord> =
        rows.iter().map(|r| (r.file_name.as_str(), r)).collect();
    let on_disk: HashSet<&str> = files.iter().map(|f| f.file_name.as_str()).collect();

    for r in rows
        .iter()
        .filter(|r| !on_disk.contains(r.file_name.as_str()))
    {
        let (i, f) = (inst.id.clone(), r.file_name.clone());
        with_db(&state, move |db| db.remove_mod(&i, &f)).await?;
    }

    let mut unknown: Vec<&ModFile> = Vec::new();
    for f in &files {
        match by_name.get(f.file_name.as_str()) {
            Some(r) if r.sha512.eq_ignore_ascii_case(&f.sha512) => {
                if r.enabled != f.enabled {
                    let (i, n, e) = (inst.id.clone(), f.file_name.clone(), f.enabled);
                    with_db(&state, move |db| db.set_mod_enabled(&i, &n, e)).await?;
                }
            }
            _ => unknown.push(f),
        }
    }

    if !unknown.is_empty() {
        let c = CancellationToken::new();
        let hashes: Vec<String> = unknown.iter().map(|f| f.sha512.clone()).collect();
        let known = match state.modrinth.versions_by_hash(&hashes, &c).await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(error = %e, "could not identify mods on Modrinth; recording as local");
                HashMap::new()
            }
        };
        for f in unknown {
            let v = known.get(&f.sha512);
            let proj = match v {
                Some(v) => state.modrinth.project(&v.project_id, &c).await.ok(),
                None => None,
            };
            let rec = ModRecord {
                instance_id: inst.id.clone(),
                file_name: f.file_name.clone(),
                source: if v.is_some() {
                    ModSource::Modrinth
                } else {
                    ModSource::Local
                },
                project_id: v.map(|v| v.project_id.clone()),
                version_id: v.map(|v| v.id.clone()),
                title: proj
                    .as_ref()
                    .map(|p| p.title.clone())
                    .unwrap_or_else(|| stem(&f.file_name)),
                version_number: v.map(|v| v.version_number.clone()),
                icon_url: proj.and_then(|p| p.icon_url),
                sha1: f.sha1.clone(),
                sha512: f.sha512.clone(),
                size: f.size as i64,
                enabled: f.enabled,
                required_by: by_name
                    .get(f.file_name.as_str())
                    .and_then(|r| r.required_by.clone()),
                installed_at: 0,
            };
            with_db(&state, move |db| db.upsert_mod(&rec)).await?;
        }
    }
    let id = inst.id.clone();
    with_db(&state, move |db| db.list_mods(&id)).await
}

fn validate_file_name(name: &str) -> Result<()> {
    if nc_launcher::modrinth::api::valid_mod_file_name(name) {
        Ok(())
    } else {
        Err(invalid("invalid mod file name"))
    }
}

#[tauri::command]
pub async fn set_mod_enabled(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
    enabled: bool,
) -> Result<()> {
    validate_file_name(&file_name)?;
    let (inst, _, dir) = ctx(&state, &instance_id).await?;
    ensure_not_running(&state, &inst.id)?;
    let n = file_name.clone();
    tauri::async_runtime::spawn_blocking(move || mods::set_enabled(&dir, &n, enabled))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
    with_db(&state, move |db| {
        db.set_mod_enabled(&inst.id, &file_name, enabled)
    })
    .await
}

#[tauri::command]
pub async fn remove_mod(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
) -> Result<()> {
    validate_file_name(&file_name)?;
    let (inst, _, dir) = ctx(&state, &instance_id).await?;
    ensure_not_running(&state, &inst.id)?;
    let n = file_name.clone();
    tauri::async_runtime::spawn_blocking(move || mods::remove(&dir, &n))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
    tracing::info!(instance = %inst.id, file = %file_name, "mod removed");
    with_db(&state, move |db| db.remove_mod(&inst.id, &file_name)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdate {
    file_name: String,
    current: Option<String>,
    version_id: String,
    version_number: String,
}

async fn updates_for(
    state: &AppState,
    inst: &Instance,
    loader: &str,
    only: Option<&HashSet<String>>,
) -> Result<Vec<(ModRecord, Version)>> {
    let id = inst.id.clone();
    let rows: Vec<ModRecord> = with_db(state, move |db| db.list_mods(&id))
        .await?
        .into_iter()
        .filter(|r| {
            r.source == ModSource::Modrinth && only.is_none_or(|o| o.contains(&r.file_name))
        })
        .collect();
    let hashes: Vec<String> = rows.iter().map(|r| r.sha512.clone()).collect();
    let latest = state
        .modrinth
        .latest_by_hash(
            &hashes,
            loader,
            &inst.game_version,
            &CancellationToken::new(),
        )
        .await
        .map_err(LauncherError::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let v = latest.get(&r.sha512)?.clone();
            (Some(&v.id) != r.version_id.as_ref() && v.supports(loader, &inst.game_version))
                .then_some((r, v))
        })
        .collect())
}

#[tauri::command]
pub async fn check_mod_updates(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Vec<ModUpdate>> {
    let (inst, loader, _) = ctx(&state, &instance_id).await?;
    Ok(updates_for(&state, &inst, loader, None)
        .await?
        .into_iter()
        .map(|(r, v)| ModUpdate {
            file_name: r.file_name,
            current: r.version_number,
            version_id: v.id,
            version_number: v.version_number,
        })
        .collect())
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSummary {
    updated: u32,
    /// Titles of dependencies that newer versions now require and that were installed.
    added_dependencies: Vec<String>,
    /// Titles (or ids) of newly required dependencies with no compatible version.
    missing_dependencies: Vec<String>,
}

/// Update the named mods to their latest compatible versions. The new file
/// is downloaded and verified before the old one is removed; disabled mods
/// stay disabled. Dependencies a newer version starts to require are
/// resolved like an install plan and installed too.
#[tauri::command]
pub async fn update_mods(
    state: State<'_, AppState>,
    instance_id: String,
    file_names: Vec<String>,
) -> Result<UpdateSummary> {
    for f in &file_names {
        validate_file_name(f)?;
    }
    let (inst, loader, dir) = ctx(&state, &instance_id).await?;
    ensure_not_running(&state, &inst.id)?;
    let only: HashSet<String> = file_names.into_iter().collect();
    let dl = state.meta().dl;
    let c = CancellationToken::new();
    let id = inst.id.clone();
    let mut installed: HashSet<String> = with_db(&state, move |db| db.list_mods(&id))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let mut summary = UpdateSummary::default();
    let mut n = 0;
    for (old, v) in updates_for(&state, &inst, loader, Some(&only)).await? {
        // Resolve first, so a network failure leaves the old version in place.
        let deps = plan::plan(
            &state.modrinth,
            v.clone(),
            loader,
            &inst.game_version,
            &installed,
            &c,
        )
        .await
        .map_err(LauncherError::from)?;
        let new = mods::install_version(&dl, &dir, &v, &c).await?;
        if new.file_name != old.file_name {
            mods::remove(&dir, &old.file_name)?;
            let (i, f) = (inst.id.clone(), old.file_name.clone());
            with_db(&state, move |db| db.remove_mod(&i, &f)).await?;
        }
        if !old.enabled {
            mods::set_enabled(&dir, &new.file_name, false)?;
        }
        let rec = ModRecord {
            file_name: new.file_name.clone(),
            version_id: Some(v.id.clone()),
            version_number: Some(v.version_number.clone()),
            sha1: new.sha1,
            sha512: new.sha512,
            size: new.size as i64,
            installed_at: 0,
            ..old
        };
        with_db(&state, move |db| db.upsert_mod(&rec)).await?;
        n += 1;

        for dep in deps.install.into_iter().filter(|p| p.required_by.is_some()) {
            let rec = install_and_record(&state, &inst.id, &dir, &dep.version, dep.required_by, &c)
                .await?;
            installed.insert(dep.version.project_id);
            summary.added_dependencies.push(rec.title);
        }
        for pid in deps.missing {
            let title = match state.modrinth.project(&pid, &c).await {
                Ok(p) => p.title,
                Err(_) => pid,
            };
            if !summary.missing_dependencies.contains(&title) {
                summary.missing_dependencies.push(title);
            }
        }
    }
    summary.updated = n;
    tracing::info!(
        instance = %inst.id,
        updated = n,
        added = summary.added_dependencies.len(),
        missing = summary.missing_dependencies.len(),
        "mods updated"
    );
    Ok(summary)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    record: ModRecord,
    warnings: Vec<String>,
}

/// Pick a .jar with the native dialog (in Rust: the WebView never supplies a
/// path) and copy it into the mods folder after validation.
#[tauri::command]
pub async fn import_local_mod(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<Option<Imported>> {
    let (inst, loader, dir) = ctx(&state, &instance_id).await?;
    ensure_not_running(&state, &inst.id)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Add a mod file")
        .add_filter("Minecraft mod", &["jar"])
        .pick_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let Some(src) = rx
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
    else {
        return Ok(None);
    };
    let d = dir.clone();
    let (file, info) =
        tauri::async_runtime::spawn_blocking(move || mods::import_local(&d, &src, loader))
            .await
            .map_err(|e| CmdError::new("internal", e.to_string()))??;

    let mut warnings = Vec::new();
    let known = state
        .modrinth
        .versions_by_hash(
            std::slice::from_ref(&file.sha512),
            &CancellationToken::new(),
        )
        .await
        .unwrap_or_default();
    let v = known.get(&file.sha512);
    if v.is_none() {
        warnings.push("This file is not from Modrinth, so Night Client cannot verify where it came from or check it for updates.".to_owned());
    }
    if info.loader.is_none() {
        warnings.push(format!(
            "The file does not say which loader it is for; it may not work with {loader}."
        ));
    }
    let rec = ModRecord {
        instance_id: inst.id.clone(),
        file_name: file.file_name.clone(),
        source: if v.is_some() {
            ModSource::Modrinth
        } else {
            ModSource::Local
        },
        project_id: v.map(|v| v.project_id.clone()),
        version_id: v.map(|v| v.id.clone()),
        title: info.mod_id.clone().unwrap_or_else(|| stem(&file.file_name)),
        version_number: v.map(|v| v.version_number.clone()),
        icon_url: None,
        sha1: file.sha1,
        sha512: file.sha512,
        size: file.size as i64,
        enabled: true,
        required_by: None,
        installed_at: 0,
    };
    let r = rec.clone();
    with_db(&state, move |db| db.upsert_mod(&r)).await?;
    tracing::info!(instance = %inst.id, file = %rec.file_name, verified = v.is_some(), "local mod imported");
    Ok(Some(Imported {
        record: rec,
        warnings,
    }))
}
