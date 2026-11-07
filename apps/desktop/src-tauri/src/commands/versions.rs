use std::sync::Arc;

use nc_core::{manifest::VersionKind, rules::Environment};
use nc_launcher::{
    CancellationToken,
    install::{self, InstallProgress},
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{CmdError, Result, validate_version_id};
use crate::state::{AppState, lock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionItem {
    id: String,
    kind: VersionKind,
    release_time: String,
    installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionList {
    latest_release: String,
    latest_snapshot: String,
    versions: Vec<VersionItem>,
    from_cache: bool,
}

#[tauri::command]
pub async fn list_versions(
    state: State<'_, AppState>,
    include_snapshots: bool,
) -> Result<VersionList> {
    let (m, from_cache) = state
        .meta()
        .version_manifest(&CancellationToken::new())
        .await?;
    let layout = state.layout.clone();
    let versions = m
        .versions
        .into_iter()
        .filter(|v| {
            v.kind == VersionKind::Release || (include_snapshots && v.kind == VersionKind::Snapshot)
        })
        .map(|v| VersionItem {
            installed: layout.installed_marker(&v.id).is_ok_and(|p| p.is_file()),
            id: v.id,
            kind: v.kind,
            release_time: v.release_time,
        })
        .collect();
    Ok(VersionList {
        latest_release: m.latest.release,
        latest_snapshot: m.latest.snapshot,
        versions,
        from_cache,
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    id: String,
    #[serde(flatten)]
    progress: InstallProgress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinishedEvent {
    id: String,
    ok: bool,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    downloaded_files: u64,
    skipped_files: u64,
    downloaded_bytes: u64,
}

#[tauri::command]
pub async fn install_version(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<InstallResult> {
    validate_version_id(&id)?;
    let token = {
        let mut slot = lock(&state.install);
        if let Some((running, _)) = slot.as_ref() {
            return Err(CmdError::new(
                "busy",
                format!("{running} is already being installed"),
            ));
        }
        let t = CancellationToken::new();
        *slot = Some((id.clone(), t.clone()));
        t
    };
    let progress_id = id.clone();
    let emitter = app.clone();
    let on_progress = Arc::new(move |progress: InstallProgress| {
        let _ = emitter.emit(
            "install://progress",
            ProgressEvent {
                id: progress_id.clone(),
                progress,
            },
        );
    });
    let meta = state.meta();
    let r = install::install_vanilla(&meta, &id, &Environment::windows_x64(), &token, on_progress)
        .await;
    *lock(&state.install) = None;
    let _ = app.emit(
        "install://finished",
        FinishedEvent {
            id: id.clone(),
            ok: r.is_ok(),
            error: r.as_ref().err().map(ToString::to_string),
        },
    );
    match &r {
        Ok(_) => tracing::info!(%id, "install finished"),
        Err(e) => tracing::warn!(%id, error = %e, "install failed"),
    }
    let r = r?;
    Ok(InstallResult {
        downloaded_files: r.files.downloaded_files,
        skipped_files: r.files.skipped_files,
        downloaded_bytes: r.files.downloaded_bytes,
    })
}

#[tauri::command]
pub fn cancel_install(state: State<'_, AppState>) {
    if let Some((id, t)) = lock(&state.install).as_ref() {
        tracing::info!(%id, "install cancelled by user");
        t.cancel();
    }
}

/// Version currently being installed, if any (for UI state after reload).
#[tauri::command]
pub fn current_install(state: State<'_, AppState>) -> Option<String> {
    lock(&state.install).as_ref().map(|(id, _)| id.clone())
}
