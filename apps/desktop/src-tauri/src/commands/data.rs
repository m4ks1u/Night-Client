//! Data-folder location: show it, and move the game data somewhere else.

use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use nc_launcher::relocate;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{CmdError, Result, invalid, with_db};
use crate::state::{AppState, lock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFolder {
    path: String,
    default_path: String,
    is_default: bool,
    /// Size of the game data that a move would copy (bytes).
    size_bytes: u64,
}

#[tauri::command]
pub async fn data_folder_info(state: State<'_, AppState>) -> Result<DataFolder> {
    let root = state.layout.root().to_path_buf();
    let r = root.clone();
    let size_bytes = tauri::async_runtime::spawn_blocking(move || relocate::managed_size(&r))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?;
    Ok(DataFolder {
        path: root.display().to_string(),
        default_path: state.home.display().to_string(),
        is_default: root == state.home,
        size_bytes,
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    done_bytes: u64,
    total_bytes: u64,
}

/// Move the game data to `target` (or back to the default folder when
/// `None`), then restart. Refused while a game runs or downloads are queued.
#[tauri::command]
pub async fn relocate_data(
    app: AppHandle,
    state: State<'_, AppState>,
    target: Option<String>,
) -> Result<()> {
    let from = state.layout.root().to_path_buf();
    let home = state.home.clone();
    let input = target.unwrap_or_else(|| home.display().to_string());
    if input.len() > 1024 {
        return Err(invalid("path is too long"));
    }
    let to = relocate::validate_target(&from, &home, &input)?;

    // Claim the flag first, so nothing new starts while we check what runs.
    if state
        .relocating
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(CmdError::new(
            "busy",
            "The data folder is already being moved.",
        ));
    }
    let guard = ClearOnDrop(&state.relocating);

    if !lock(&state.games).is_empty() {
        return Err(CmdError::new(
            "busy",
            "Close running games before moving the data folder.",
        ));
    }
    {
        let q = lock(&state.queue).snapshot();
        if q.active.is_some() || !q.pending.is_empty() {
            return Err(CmdError::new(
                "busy",
                "Wait for downloads to finish before moving the data folder.",
            ));
        }
    }
    // Custom game folders are stored as absolute paths; one inside a managed
    // folder would break when that folder moves.
    let blocked = with_db(&state, |db| db.list_instances())
        .await?
        .into_iter()
        .filter_map(|i| i.game_dir.map(|g| (i.name, PathBuf::from(g))))
        .find(|(_, g)| inside_managed(&from, g));
    if let Some((name, _)) = blocked {
        return Err(CmdError::new(
            "busy",
            format!(
                "“{name}” uses a game folder inside the data folder. Choose another game folder for it first."
            ),
        ));
    }

    tracing::info!(from = %from.display(), to = %to.display(), "moving data folder");

    let emitter = app.clone();
    let (f, t, h) = (from.clone(), to.clone(), home.clone());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let last = std::sync::atomic::AtomicU64::new(0);
        let progress = |done: u64, total: u64| {
            // At most ~200 events for the whole move.
            let step = (total / 200).max(1);
            if done == total || done - last.load(Ordering::Relaxed) >= step {
                last.store(done, Ordering::Relaxed);
                let _ = emitter.emit(
                    "data://relocate-progress",
                    Progress {
                        done_bytes: done,
                        total_bytes: total,
                    },
                );
            }
        };
        relocate::relocate(&f, &t, false, &progress, || relocate::save_location(&h, &t))
    })
    .await
    .map_err(|e| CmdError::new("internal", e.to_string()));

    match result {
        Ok(Ok(())) => {
            // Everything now points at the new folder; keep refusing work
            // until the restart picks it up.
            std::mem::forget(guard);
            tracing::info!(to = %to.display(), "data folder moved; restarting");
            app.restart();
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "data folder move failed and was rolled back");
            Err(e.into())
        }
        Err(e) => Err(e),
    }
}

/// Clears the relocation flag unless forgotten (on success).
struct ClearOnDrop<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn inside_managed(root: &Path, p: &Path) -> bool {
    let canon = |q: &Path| std::fs::canonicalize(q).unwrap_or_else(|_| q.to_path_buf());
    let p = canon(p);
    relocate::MANAGED
        .iter()
        .any(|d| p.starts_with(canon(&root.join(d))))
}
