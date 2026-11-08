use nc_store::{Instance, InstancePatch, Loader, NewInstance};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::{CmdError, Result, validate_instance_id, validate_version_id, with_db};
use crate::state::{AppState, lock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    #[serde(flatten)]
    instance: Instance,
    installed: bool,
    running: bool,
}

fn view(state: &AppState, instance: Instance) -> InstanceView {
    InstanceView {
        installed: state
            .layout
            .installed_marker(&instance.game_version)
            .is_ok_and(|p| p.is_file()),
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInstance {
    name: String,
    game_version: String,
    max_memory_mb: Option<u32>,
}

#[tauri::command]
pub async fn create_instance(
    state: State<'_, AppState>,
    input: CreateInstance,
) -> Result<InstanceView> {
    validate_version_id(&input.game_version)?;
    let instances_dir = state.layout.instances_dir();
    let new = NewInstance {
        name: input.name,
        game_version: input.game_version,
        loader: Loader::Vanilla,
        loader_version: None,
        max_memory_mb: input.max_memory_mb,
    };
    let created = with_db(&state, move |db| {
        db.create_instance(new, |dir| instances_dir.join(dir).exists())
    })
    .await?;
    nc_launcher::instance::ensure_game_dir(&state.layout, &created.dir_name)?;
    tracing::info!(id = %created.id, version = %created.game_version, "instance created");
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
        java_path: None,
    };
    let i = with_db(&state, move |db| db.update_instance(&id, p)).await?;
    Ok(view(&state, i))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deleted {
    trash_path: Option<String>,
}

#[tauri::command]
pub async fn delete_instance(state: State<'_, AppState>, id: String) -> Result<Deleted> {
    validate_instance_id(&id)?;
    if lock(&state.games).contains_key(&id) {
        return Err(CmdError::new(
            "busy",
            "stop the game before deleting this instance",
        ));
    }
    let gone = with_db(&state, move |db| db.delete_instance(&id)).await?;
    // Row first, then folder: if the move fails the files are still on disk
    // and nothing references a missing folder.
    let trash = match nc_launcher::instance::move_to_trash(&state.layout, &gone.dir_name) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, dir = %gone.dir_name, "instance folder could not be moved to trash");
            None
        }
    };
    tracing::info!(id = %gone.id, trash = ?trash, "instance deleted");
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
    let dir = nc_launcher::instance::ensure_game_dir(&state.layout, &i.dir_name)?;
    super::app::open_dir(&app, &dir)
}
