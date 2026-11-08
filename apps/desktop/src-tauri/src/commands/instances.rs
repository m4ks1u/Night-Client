use nc_core::loader::{InstallKey, LoaderKind};
use nc_launcher::{CancellationToken, loaders::LoaderVersion};
use nc_store::{Instance, InstancePatch, Loader, NewInstance};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::{CmdError, Result, invalid, validate_instance_id, validate_version_id, with_db};
use crate::state::{AppState, lock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    #[serde(flatten)]
    instance: Instance,
    /// `vanilla:<mc>` / `fabric:<mc>:<v>` / `quilt:<mc>:<v>` / `neoforge:<mc>:<v>`: the download-queue key.
    install_key: Option<String>,
    /// Human label, e.g. "Fabric 0.17.2 · 1.21.1".
    label: String,
    installed: bool,
    running: bool,
}

fn to_kind(l: Loader) -> LoaderKind {
    match l {
        Loader::Vanilla => LoaderKind::Vanilla,
        Loader::Fabric => LoaderKind::Fabric,
        Loader::Quilt => LoaderKind::Quilt,
        Loader::Neoforge => LoaderKind::NeoForge,
    }
}

fn from_kind(k: LoaderKind) -> Loader {
    match k {
        LoaderKind::Vanilla => Loader::Vanilla,
        LoaderKind::Fabric => Loader::Fabric,
        LoaderKind::Quilt => Loader::Quilt,
        LoaderKind::NeoForge => Loader::Neoforge,
    }
}

/// The install key for an instance; `None` if its stored data is invalid.
pub fn install_key(i: &Instance) -> Option<InstallKey> {
    InstallKey::new(
        to_kind(i.loader),
        &i.game_version,
        i.loader_version.as_deref(),
    )
    .ok()
}

pub(crate) fn view(state: &AppState, instance: Instance) -> InstanceView {
    let key = install_key(&instance);
    InstanceView {
        installed: key
            .as_ref()
            .and_then(|k| state.layout.installed_marker(&k.launch_id()).ok())
            .is_some_and(|p| p.is_file()),
        label: key
            .as_ref()
            .map(InstallKey::label)
            .unwrap_or_else(|| instance.game_version.clone()),
        install_key: key.map(|k| k.to_key()),
        running: lock(&state.games).contains_key(&instance.id),
        instance,
    }
}

#[tauri::command]
pub async fn list_instances(state: State<'_, AppState>) -> Result<Vec<InstanceView>> {
    let list = with_db(&state, |db| db.list_instances()).await?;
    Ok(list.into_iter().map(|i| view(&state, i)).collect())
}

#[tauri::command]
pub async fn get_instance(state: State<'_, AppState>, id: String) -> Result<InstanceView> {
    validate_instance_id(&id)?;
    let i = with_db(&state, move |db| db.get_instance(&id)).await?;
    Ok(view(&state, i))
}

/// Loader versions for a Minecraft version, newest first.
#[tauri::command]
pub async fn loader_versions(
    state: State<'_, AppState>,
    loader: LoaderKind,
    minecraft: String,
) -> Result<Vec<LoaderVersion>> {
    validate_version_id(&minecraft)?;
    Ok(
        nc_launcher::loaders::versions(
            &state.meta(),
            loader,
            &minecraft,
            &CancellationToken::new(),
        )
        .await?,
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInstance {
    name: String,
    game_version: String,
    #[serde(default = "vanilla")]
    loader: LoaderKind,
    #[serde(default)]
    loader_version: Option<String>,
    max_memory_mb: Option<u32>,
}

fn vanilla() -> LoaderKind {
    LoaderKind::Vanilla
}

#[tauri::command]
pub async fn create_instance(
    state: State<'_, AppState>,
    input: CreateInstance,
) -> Result<InstanceView> {
    super::ensure_not_relocating(&state)?;
    validate_version_id(&input.game_version)?;
    let key = InstallKey::new(
        input.loader,
        &input.game_version,
        input.loader_version.as_deref(),
    )
    .map_err(|_| invalid("choose a valid loader version"))?;
    let instances_dir = state.layout.instances_dir();
    let new = NewInstance {
        name: input.name,
        game_version: key.minecraft.clone(),
        loader: from_kind(key.kind),
        loader_version: key.loader_version.clone(),
        max_memory_mb: input.max_memory_mb,
    };
    let created = with_db(&state, move |db| {
        db.create_instance(new, |dir| instances_dir.join(dir).exists())
    })
    .await?;
    nc_launcher::instance::ensure_game_dir(&state.layout, &created.dir_name)?;
    tracing::info!(id = %created.id, key = %key.to_key(), "instance created");
    Ok(view(&state, created))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateInstance {
    name: Option<String>,
    /// Present-and-null resets to the launcher default.
    #[serde(default, with = "double_option")]
    max_memory_mb: Option<Option<u32>>,
}

mod double_option {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u32>>, D::Error> {
        Option::<u32>::deserialize(d).map(Some)
    }
}

#[tauri::command]
pub async fn update_instance(
    state: State<'_, AppState>,
    id: String,
    patch: UpdateInstance,
) -> Result<InstanceView> {
    validate_instance_id(&id)?;
    let p = InstancePatch {
        name: patch.name,
        max_memory_mb: patch.max_memory_mb,
        ..Default::default()
    };
    let i = with_db(&state, move |db| db.update_instance(&id, p)).await?;
    Ok(view(&state, i))
}

/// Change the loader version of a Fabric/NeoForge instance. The new
/// version is installed on next play (or via Verify/Download).
#[tauri::command]
pub async fn set_instance_loader_version(
    state: State<'_, AppState>,
    id: String,
    version: String,
) -> Result<InstanceView> {
    validate_instance_id(&id)?;
    super::ensure_not_relocating(&state)?;
    if lock(&state.games).contains_key(&id) {
        return Err(CmdError::new(
            "busy",
            "stop the game before changing the loader",
        ));
    }
    let lookup = id.clone();
    let current = with_db(&state, move |db| db.get_instance(&lookup)).await?;
    InstallKey::new(
        to_kind(current.loader),
        &current.game_version,
        Some(&version),
    )
    .map_err(|_| invalid("invalid loader version"))?;
    let p = InstancePatch {
        loader_version: Some(version),
        ..Default::default()
    };
    let i = with_db(&state, move |db| db.update_instance(&id, p)).await?;
    Ok(view(&state, i))
}

/// Set (or clear with `null`) a custom game folder. Validated: absolute, not
/// a system or Night Client-managed folder. Files are not moved.
#[tauri::command]
pub async fn set_instance_game_dir(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
) -> Result<InstanceView> {
    validate_instance_id(&id)?;
    super::ensure_not_relocating(&state)?;
    if lock(&state.games).contains_key(&id) {
        return Err(CmdError::new(
            "busy",
            "stop the game before changing its folder",
        ));
    }
    let stored = match path {
        None => None,
        Some(p) => {
            if p.len() > 1024 {
                return Err(invalid("path is too long"));
            }
            let layout = state.layout.clone();
            let canon = tauri::async_runtime::spawn_blocking(move || {
                nc_launcher::instance::validate_custom_game_dir(&layout, &p)
            })
            .await
            .map_err(|e| CmdError::new("internal", e.to_string()))??;
            Some(
                canon
                    .display()
                    .to_string()
                    .trim_start_matches(r"\\?\")
                    .to_owned(),
            )
        }
    };
    let p = InstancePatch {
        game_dir: Some(stored),
        ..Default::default()
    };
    let i = with_db(&state, move |db| db.update_instance(&id, p)).await?;
    tracing::info!(instance = %i.id, custom = i.game_dir.is_some(), "game folder changed");
    Ok(view(&state, i))
}

/// Native folder picker (runs in Rust; the WebView gets no file-system access).
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a game folder")
        .pick_folder(move |p| {
            let _ = tx.send(
                p.and_then(|p| p.into_path().ok())
                    .map(|p| p.display().to_string()),
            );
        });
    rx.await
        .map_err(|e| CmdError::new("internal", e.to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deleted {
    trash_path: Option<String>,
}

#[tauri::command]
pub async fn delete_instance(state: State<'_, AppState>, id: String) -> Result<Deleted> {
    validate_instance_id(&id)?;
    super::ensure_not_relocating(&state)?;
    if lock(&state.games).contains_key(&id) {
        return Err(CmdError::new(
            "busy",
            "stop the game before deleting this instance",
        ));
    }
    let gone = with_db(&state, move |db| db.delete_instance(&id)).await?;
    // Row first, then folder: if the move fails the files are still on disk
    // and nothing references a missing folder. A custom game folder belongs
    // to the user and is never moved; only the default folder (which then
    // holds just extracted natives) goes to trash.
    let trash = match nc_launcher::instance::move_to_trash(&state.layout, &gone.dir_name) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, dir = %gone.dir_name, "instance folder could not be moved to trash");
            None
        }
    };
    tracing::info!(id = %gone.id, trash = ?trash, custom_dir_kept = gone.game_dir.is_some(), "instance deleted");
    Ok(Deleted {
        trash_path: trash.map(|p| p.display().to_string()),
    })
}

#[tauri::command]
pub async fn open_instance_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<()> {
    validate_instance_id(&id)?;
    let i = with_db(&state, move |db| db.get_instance(&id)).await?;
    let dir = match &i.game_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => nc_launcher::instance::ensure_game_dir(&state.layout, &i.dir_name)?,
    };
    super::app::open_dir(&app, &dir)
}

/// Set an instance's extra JVM arguments from user text. The text is split
/// without a shell and validated against an allowlist; an empty string clears them.
#[tauri::command]
pub async fn set_instance_jvm_args(
    state: State<'_, AppState>,
    id: String,
    args: String,
) -> Result<InstanceView> {
    validate_instance_id(&id)?;
    if args.len() > 8192 {
        return Err(invalid("JVM arguments are too long"));
    }
    let parsed = nc_core::jvmargs::parse(&args)
        .map_err(|e| CmdError::new("invalid_jvm_args", e.to_string()))?;
    let patch = InstancePatch {
        jvm_args: Some(parsed),
        ..Default::default()
    };
    let i = with_db(&state, move |db| db.update_instance(&id, patch)).await?;
    Ok(view(&state, i))
}
