//! Modrinth discovery and per-instance content management: mods, resource
//! packs and shader packs. Every command takes an optional `kind` (default:
//! mods); each kind has its own folder and its own rows.

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
use nc_store::{ContentKind, Instance, Loader, ModRecord, ModSource};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::{CmdError, Result, invalid, validate_instance_id, with_db};
use crate::state::{AppState, lock};

/// What a command works on: the instance, the Modrinth loader tags accepted
/// for `kind` there (the instance's own first), and the folder for `kind`.
/// Mods are refused on vanilla instances; packs work everywhere.
struct Ctx {
    inst: Instance,
    kind: ContentKind,
    loaders: &'static [&'static str],
    dir: PathBuf,
}

fn loader_name(l: Loader) -> &'static str {
    match l {
        Loader::Vanilla => "vanilla",
        Loader::Fabric => "fabric",
        Loader::Quilt => "quilt",
        Loader::Neoforge => "neoforge",
        Loader::Forge => "forge",
    }
}

async fn ctx(state: &AppState, id: &str, kind: Option<ContentKind>) -> Result<Ctx> {
    validate_instance_id(id)?;
    let kind = kind.unwrap_or_default();
    let lookup = id.to_owned();
    let inst = with_db(state, move |db| db.get_instance(&lookup)).await?;
    let loader = loader_name(inst.loader);
    if !kind.available_for(loader) {
        return Err(CmdError::new(
            "no_loader",
            "Vanilla instances can't use mods. Create a Fabric, Quilt, NeoForge or Forge instance.",
        ));
    }
    let game_dir = match &inst.game_dir {
        Some(d) => PathBuf::from(d),
        None => state
            .layout
            .instance_dir(&inst.dir_name)
            .map_err(LauncherError::from)?,
    };
    Ok(Ctx {
        loaders: kind.modrinth_loaders(loader),
        dir: mods::content_dir(&game_dir, kind),
        inst,
        kind,
    })
}

fn what(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Mod => "mod",
        ContentKind::ResourcePack => "resource pack",
        ContentKind::Shader => "shader pack",
    }
}

fn ensure_not_running(state: &AppState, id: &str) -> Result<()> {
    super::ensure_not_relocating(state)?;
    if lock(&state.games).contains_key(id) {
        Err(CmdError::new(
            "busy",
            "Close the game before changing its mods or packs.",
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
    kind: Option<ContentKind>,
) -> Result<SearchView> {
    if query.len() > 200 {
        return Err(invalid("search text is too long"));
    }
    if category.as_deref().is_some_and(|c| !valid_id(c)) {
        return Err(invalid("invalid category"));
    }
    let Ctx {
        inst,
        kind,
        loaders,
        ..
    } = ctx(&state, &instance_id, kind).await?;
    let id = inst.id.clone();
    let installed: HashSet<String> = with_db(&state, move |db| db.list_content(&id, kind))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let r = state
        .modrinth
        .search(
            &SearchQuery {
                query: query.trim(),
                kind,
                loaders,
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
pub fn open_modrinth_page(
    app: AppHandle,
    slug: String,
    kind: Option<ContentKind>,
    modpack: Option<bool>,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    if !valid_id(&slug) {
        return Err(invalid("invalid project"));
    }
    let kind = if modpack == Some(true) {
        "modpack"
    } else {
        kind.unwrap_or_default().as_str()
    };
    app.opener()
        .open_url(format!("https://modrinth.com/{kind}/{slug}"), None::<&str>)
        .map_err(|e| CmdError::new("io", e.to_string()))
}

#[tauri::command]
pub async fn modrinth_categories(
    state: State<'_, AppState>,
    kind: Option<ContentKind>,
) -> Result<Vec<Category>> {
    Ok(state
        .modrinth
        .categories(kind.unwrap_or_default(), &CancellationToken::new())
        .await
        .map_err(LauncherError::from)?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    #[serde(flatten)]
    plan: Plan,
    /// One plan per optional dependency that has a compatible version: the
    /// optional mod first, then what it requires beyond the main plan.
    options: Vec<Plan>,
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
    kind: Option<ContentKind>,
) -> Result<PlanView> {
    if !valid_id(&project_id) {
        return Err(invalid("invalid project"));
    }
    let Ctx {
        inst,
        kind,
        loaders,
        ..
    } = ctx(&state, &instance_id, kind).await?;
    let loader = loader_name(inst.loader);
    let c = CancellationToken::new();
    let mr = &state.modrinth;
    let versions = mr
        .project_versions(&project_id, loaders, &inst.game_version, &c)
        .await
        .map_err(LauncherError::from)?;
    let root = pick_best(versions, loaders, &inst.game_version).ok_or(CmdError::new(
        "mod_incompatible",
        if kind == ContentKind::Mod {
            format!(
                "This mod has no version for {loader} on Minecraft {}.",
                inst.game_version
            )
        } else {
            format!(
                "This {} has no version for Minecraft {}{}.",
                what(kind),
                inst.game_version,
                if kind == ContentKind::Shader && loader == "vanilla" {
                    " without a shader mod (Iris or OptiFine)"
                } else {
                    ""
                }
            )
        },
    ))?;
    if kind != ContentKind::Mod {
        // Packs are single files: no dependency resolution. (Shader packs
        // may list Iris as a dependency; that is a mod, installed separately.)
        let mut titles = HashMap::new();
        if let Ok(proj) = mr.project(&root.project_id, &c).await {
            titles.insert(root.project_id.clone(), proj.title);
        }
        return Ok(PlanView {
            plan: Plan {
                install: vec![plan::PlannedMod {
                    version: root,
                    required_by: None,
                }],
                already_installed: Vec::new(),
                missing: Vec::new(),
                conflicts: Vec::new(),
                optional: Vec::new(),
            },
            options: Vec::new(),
            titles,
        });
    }
    let id = inst.id.clone();
    let installed: HashSet<String> = with_db(&state, move |db| db.list_mods(&id))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let p = plan::plan(mr, root, loaders, &inst.game_version, &installed, &c)
        .await
        .map_err(LauncherError::from)?;

    // Optional dependencies: resolve each against what is installed or
    // already planned, so a ticked option adds only what it needs.
    let mut have = installed.clone();
    have.extend(p.install.iter().map(|m| m.version.project_id.clone()));
    let mut options = Vec::new();
    for pid in &p.optional {
        let Ok(vs) = mr
            .project_versions(pid, loaders, &inst.game_version, &c)
            .await
        else {
            continue;
        };
        let Some(v) = pick_best(vs, loaders, &inst.game_version) else {
            continue;
        };
        if let Ok(sub) = plan::plan(mr, v, loaders, &inst.game_version, &have, &c).await {
            options.push(sub);
        }
    }

    let mut titles = HashMap::new();
    let ids: Vec<String> = p
        .install
        .iter()
        .map(|m| m.version.project_id.clone())
        .chain(p.missing.iter().cloned())
        .chain(p.conflicts.iter().cloned())
        .chain(p.already_installed.iter().cloned())
        .chain(options.iter().flat_map(|o| {
            o.install
                .iter()
                .map(|m| m.version.project_id.clone())
                .chain(o.missing.iter().cloned())
                .chain(o.conflicts.iter().cloned())
        }))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    for pid in ids {
        if let Ok(proj) = mr.project(&pid, &c).await {
            titles.insert(pid, proj.title);
        }
    }
    Ok(PlanView {
        plan: p,
        options,
        titles,
    })
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
    kind: Option<ContentKind>,
) -> Result<Vec<ModRecord>> {
    if items.is_empty() || items.len() > 64 {
        return Err(invalid("nothing to install"));
    }
    let Ctx {
        inst,
        kind,
        loaders,
        dir,
    } = ctx(&state, &instance_id, kind).await?;
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
        if !v.supports(loaders, &inst.game_version) {
            return Err(CmdError::new(
                "mod_incompatible",
                format!(
                    "{} is not a {} for {} {}.",
                    v.name,
                    what(kind),
                    loader_name(inst.loader),
                    inst.game_version
                ),
            ));
        }
        done.push(
            install_and_record(&state, &inst.id, kind, &dir, &v, item.required_by, &c).await?,
        );
    }
    Ok(done)
}

/// Download and verify one Modrinth version into the folder for `kind` and
/// store its row (title and icon from the project, when reachable).
async fn install_and_record(
    state: &AppState,
    instance_id: &str,
    kind: ContentKind,
    dir: &std::path::Path,
    v: &Version,
    required_by: Option<String>,
    c: &CancellationToken,
) -> Result<ModRecord> {
    let file = mods::install_version(&state.meta().dl, dir, kind, v, c).await?;
    let proj = state.modrinth.project(&v.project_id, c).await.ok();
    let rec = ModRecord {
        instance_id: instance_id.to_owned(),
        kind,
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
    tracing::info!(instance = %instance_id, kind = kind.as_str(), file = %file.file_name, "content installed");
    Ok(rec)
}

fn stem(file_name: &str) -> String {
    file_name
        .strip_suffix(".jar")
        .or_else(|| file_name.strip_suffix(".zip"))
        .unwrap_or(file_name)
        .to_owned()
}

/// List mods, re-syncing metadata with the folder: rows for deleted files
/// are dropped, new or replaced files are identified by SHA-512 on Modrinth
/// (offline: recorded as local), and enabled state follows the disk.
#[tauri::command]
pub async fn list_mods(
    state: State<'_, AppState>,
    instance_id: String,
    kind: Option<ContentKind>,
) -> Result<Vec<ModRecord>> {
    let Ctx {
        inst, kind, dir, ..
    } = ctx(&state, &instance_id, kind).await?;
    let scan_dir = dir.clone();
    let files: Vec<ModFile> =
        tauri::async_runtime::spawn_blocking(move || mods::scan(&scan_dir, kind))
            .await
            .map_err(|e| CmdError::new("internal", e.to_string()))??;
    let id = inst.id.clone();
    let rows = with_db(&state, move |db| db.list_content(&id, kind)).await?;
    let by_name: HashMap<&str, &ModRecord> =
        rows.iter().map(|r| (r.file_name.as_str(), r)).collect();
    let on_disk: HashSet<&str> = files.iter().map(|f| f.file_name.as_str()).collect();

    for r in rows
        .iter()
        .filter(|r| !on_disk.contains(r.file_name.as_str()))
    {
        let (i, f) = (inst.id.clone(), r.file_name.clone());
        with_db(&state, move |db| db.remove_mod(&i, kind, &f)).await?;
    }

    let mut unknown: Vec<&ModFile> = Vec::new();
    for f in &files {
        match by_name.get(f.file_name.as_str()) {
            Some(r) if r.sha512.eq_ignore_ascii_case(&f.sha512) => {
                if r.enabled != f.enabled {
                    let (i, n, e) = (inst.id.clone(), f.file_name.clone(), f.enabled);
                    with_db(&state, move |db| db.set_mod_enabled(&i, kind, &n, e)).await?;
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
                tracing::warn!(error = %e, "could not identify files on Modrinth; recording as local");
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
                kind,
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
    with_db(&state, move |db| db.list_content(&id, kind)).await
}

fn validate_file_name(kind: Option<ContentKind>, name: &str) -> Result<()> {
    if nc_launcher::modrinth::api::valid_file_name(kind.unwrap_or_default(), name) {
        Ok(())
    } else {
        Err(invalid("invalid file name"))
    }
}

#[tauri::command]
pub async fn set_mod_enabled(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
    enabled: bool,
    kind: Option<ContentKind>,
) -> Result<()> {
    validate_file_name(kind, &file_name)?;
    let Ctx {
        inst, kind, dir, ..
    } = ctx(&state, &instance_id, kind).await?;
    ensure_not_running(&state, &inst.id)?;
    let n = file_name.clone();
    tauri::async_runtime::spawn_blocking(move || mods::set_enabled(&dir, kind, &n, enabled))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
    with_db(&state, move |db| {
        db.set_mod_enabled(&inst.id, kind, &file_name, enabled)
    })
    .await
}

#[tauri::command]
pub async fn remove_mod(
    state: State<'_, AppState>,
    instance_id: String,
    file_name: String,
    kind: Option<ContentKind>,
) -> Result<()> {
    validate_file_name(kind, &file_name)?;
    let Ctx {
        inst, kind, dir, ..
    } = ctx(&state, &instance_id, kind).await?;
    ensure_not_running(&state, &inst.id)?;
    let n = file_name.clone();
    tauri::async_runtime::spawn_blocking(move || mods::remove(&dir, kind, &n))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))??;
    tracing::info!(instance = %inst.id, kind = kind.as_str(), file = %file_name, "content removed");
    with_db(&state, move |db| db.remove_mod(&inst.id, kind, &file_name)).await
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
    cx: &Ctx,
    only: Option<&HashSet<String>>,
) -> Result<Vec<(ModRecord, Version)>> {
    let (inst, kind, loaders) = (&cx.inst, cx.kind, cx.loaders);
    let id = inst.id.clone();
    let rows: Vec<ModRecord> = with_db(state, move |db| db.list_content(&id, kind))
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
            loaders,
            &inst.game_version,
            &CancellationToken::new(),
        )
        .await
        .map_err(LauncherError::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let v = latest.get(&r.sha512)?.clone();
            (Some(&v.id) != r.version_id.as_ref() && v.supports(loaders, &inst.game_version))
                .then_some((r, v))
        })
        .collect())
}

#[tauri::command]
pub async fn check_mod_updates(
    state: State<'_, AppState>,
    instance_id: String,
    kind: Option<ContentKind>,
) -> Result<Vec<ModUpdate>> {
    let cx = ctx(&state, &instance_id, kind).await?;
    Ok(updates_for(&state, &cx, None)
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
    kind: Option<ContentKind>,
) -> Result<UpdateSummary> {
    for f in &file_names {
        validate_file_name(kind, f)?;
    }
    let cx = ctx(&state, &instance_id, kind).await?;
    let (inst, kind, loaders, dir) = (&cx.inst, cx.kind, cx.loaders, &cx.dir);
    ensure_not_running(&state, &inst.id)?;
    let only: HashSet<String> = file_names.into_iter().collect();
    let dl = state.meta().dl;
    let c = CancellationToken::new();
    let id = inst.id.clone();
    let mut installed: HashSet<String> = with_db(&state, move |db| db.list_content(&id, kind))
        .await?
        .into_iter()
        .filter_map(|m| m.project_id)
        .collect();
    let mut summary = UpdateSummary::default();
    let mut n = 0;
    for (old, v) in updates_for(&state, &cx, Some(&only)).await? {
        // Resolve first, so a network failure leaves the old version in place.
        // Packs have no dependencies to resolve.
        let deps = if kind == ContentKind::Mod {
            Some(
                plan::plan(
                    &state.modrinth,
                    v.clone(),
                    loaders,
                    &inst.game_version,
                    &installed,
                    &c,
                )
                .await
                .map_err(LauncherError::from)?,
            )
        } else {
            None
        };
        let new = mods::install_version(&dl, dir, kind, &v, &c).await?;
        if new.file_name != old.file_name {
            mods::remove(dir, kind, &old.file_name)?;
            let (i, f) = (inst.id.clone(), old.file_name.clone());
            with_db(&state, move |db| db.remove_mod(&i, kind, &f)).await?;
        }
        if !old.enabled {
            mods::set_enabled(dir, kind, &new.file_name, false)?;
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

        let Some(deps) = deps else {
            continue;
        };
        for dep in deps.install.into_iter().filter(|p| p.required_by.is_some()) {
            let rec = install_and_record(
                &state,
                &inst.id,
                kind,
                dir,
                &dep.version,
                dep.required_by,
                &c,
            )
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
        kind = kind.as_str(),
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
    let Ctx { inst, dir, .. } = ctx(&state, &instance_id, None).await?;
    let loader = loader_name(inst.loader);
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
        kind: ContentKind::Mod,
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
