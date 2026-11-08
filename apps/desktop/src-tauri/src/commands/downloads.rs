//! Download queue commands and the background worker that drains the queue.

use std::sync::Arc;

use nc_core::{loader::InstallKey, rules::Environment};
use nc_launcher::{
    install::InstallProgress,
    queue::{Enqueued, Job, JobKind, QueueSnapshot},
};
use nc_store::{DownloadRecord, DownloadStatus};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{CmdError, Result, invalid, with_db};
use crate::state::{AppState, lock, now_unix};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    job_id: u64,
    id: String,
    #[serde(flatten)]
    progress: InstallProgress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedEvent {
    job_id: u64,
    /// Version id.
    id: String,
    status: DownloadStatus,
    code: Option<&'static str>,
    error: Option<String>,
    repair: bool,
    repaired_count: u64,
}

fn emit_queue(app: &AppHandle, state: &AppState) {
    let snap = lock(&state.queue).snapshot();
    let _ = app.emit("downloads://queue", snap);
}

/// Queue a version install/repair. Returns the job id (an existing one if the
/// version is already queued or running).
#[tauri::command]
pub fn install_version(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<u64> {
    super::ensure_not_relocating(&state)?;
    // `id` is an install key: vanilla:<mc>, fabric:<mc>:<v> or neoforge:<mc>:<v>.
    let id = InstallKey::parse(&id)
        .map_err(|_| invalid("invalid install key"))?
        .to_key();
    let job = match lock(&state.queue).enqueue(JobKind::Version, &id, now_unix()) {
        Enqueued::Added(j) | Enqueued::AlreadyQueued(j) => j,
    };
    tracing::info!(version = %id, job, "download queued");
    emit_queue(&app, &state);
    state.queue_wake.notify_one();
    Ok(job)
}

#[tauri::command]
pub fn cancel_download(app: AppHandle, state: State<'_, AppState>, job_id: u64) -> Result<()> {
    if !lock(&state.queue).cancel(job_id) {
        return Err(CmdError::new(
            "not_found",
            "that download is no longer queued",
        ));
    }
    tracing::info!(job = job_id, "download cancelled by user");
    emit_queue(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn download_queue(state: State<'_, AppState>) -> QueueSnapshot {
    lock(&state.queue).snapshot()
}

#[tauri::command]
pub async fn download_history(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<Vec<DownloadRecord>> {
    with_db(&state, move |db| db.list_downloads(limit.clamp(1, 200))).await
}

#[tauri::command]
pub async fn clear_download_history(state: State<'_, AppState>) -> Result<usize> {
    with_db(&state, |db| db.clear_downloads()).await
}

/// Background worker: runs queued jobs one at a time for the lifetime of the app.
pub fn spawn_worker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let next = lock(&state.queue).start_next();
            let Some(job) = next else {
                state.queue_wake.notified().await;
                continue;
            };
            emit_queue(&app, &state);
            let started_at = now_unix();
            let finished = run_job(&app, &state, &job).await;
            let record = DownloadRecord {
                id: 0,
                kind: "version".into(),
                target: job.target.clone(),
                status: finished.0.status,
                repair: finished.0.repair,
                queued_at: job.queued_at,
                started_at,
                finished_at: now_unix(),
                files_downloaded: finished.1.0 as i64,
                files_verified: finished.1.1 as i64,
                bytes_downloaded: finished.1.2 as i64,
                repaired: finished.2,
                error: finished.0.error.clone(),
            };
            if let Some(db) = state.store.clone() {
                let _ =
                    tauri::async_runtime::spawn_blocking(move || db.record_download(&record)).await;
            }
            lock(&state.queue).finish(job.id);
            let _ = app.emit("install://finished", finished.0);
            emit_queue(&app, &state);
        }
    });
}

type JobOutcome = (FinishedEvent, (u64, u64, u64), Vec<String>);

async fn run_job(app: &AppHandle, state: &AppState, job: &Job) -> JobOutcome {
    let emitter = app.clone();
    let (job_id, version) = (job.id, job.target.clone());
    let on_progress = Arc::new(move |progress: InstallProgress| {
        let _ = emitter.emit(
            "install://progress",
            ProgressEvent {
                job_id,
                id: version.clone(),
                progress,
            },
        );
    });
    let key = match InstallKey::parse(&job.target) {
        Ok(k) => k,
        Err(e) => {
            return (
                FinishedEvent {
                    job_id,
                    id: job.target.clone(),
                    status: DownloadStatus::Failed,
                    code: Some("invalid_input"),
                    error: Some(e.to_string()),
                    repair: false,
                    repaired_count: 0,
                },
                (0, 0, 0),
                Vec::new(),
            );
        }
    };
    let was_installed = state
        .layout
        .installed_marker(&key.launch_id())
        .is_ok_and(|p| p.is_file());
    let r = nc_launcher::loaders::install(
        &state.meta(),
        &key,
        &Environment::windows_x64(),
        &job.cancel,
        on_progress,
    )
    .await;
    match r {
        Ok(done) => {
            tracing::info!(version = %job.target, downloaded = done.files.downloaded_files, repaired = done.repaired_count, "download job finished");
            (
                FinishedEvent {
                    job_id,
                    id: job.target.clone(),
                    status: DownloadStatus::Completed,
                    code: None,
                    error: None,
                    repair: done.was_installed,
                    repaired_count: done.repaired_count,
                },
                (
                    done.files.downloaded_files,
                    done.files.skipped_files,
                    done.files.downloaded_bytes,
                ),
                done.repaired,
            )
        }
        Err(e) => {
            let code = e.code();
            let status = if code == "cancelled" {
                DownloadStatus::Cancelled
            } else {
                DownloadStatus::Failed
            };
            tracing::warn!(version = %job.target, error = %e, "download job ended");
            (
                FinishedEvent {
                    job_id,
                    id: job.target.clone(),
                    status,
                    code: Some(code),
                    error: (status == DownloadStatus::Failed).then(|| e.to_string()),
                    repair: was_installed,
                    repaired_count: 0,
                },
                (0, 0, 0),
                Vec::new(),
            )
        }
    }
}
