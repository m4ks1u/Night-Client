use std::{sync::Arc, time::Duration};

use nc_core::launch::LaunchAuth;
use nc_launcher::{
    CancellationToken,
    instance::{LaunchRequest, prepare_launch},
    process::{self, LogLine},
    queue::JobKind,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{CmdError, Result, validate_instance_id, with_db};
use crate::state::{AppState, GameHandle, lock, now_unix};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogBatch {
    instance_id: String,
    lines: Vec<LogLine>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitEvent {
    instance_id: String,
    code: Option<i32>,
    seconds: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    instance_id: String,
    started_at: i64,
}

#[tauri::command]
pub fn running_instances(state: State<'_, AppState>) -> Vec<Running> {
    lock(&state.games)
        .iter()
        .map(|(id, g)| Running {
            instance_id: id.clone(),
            started_at: g.started_at,
        })
        .collect()
}

#[tauri::command]
pub async fn launch_instance(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    validate_instance_id(&id)?;
    super::ensure_not_relocating(&state)?;
    let lookup = id.clone();
    let (instance, settings) = with_db(&state, move |db| {
        Ok((db.get_instance(&lookup)?, db.app_settings()))
    })
    .await?;

    let session = super::account::active_session(&state).await?;
    let auth = LaunchAuth {
        player_name: session.profile.name.clone(),
        uuid: session.profile.id.clone(),
        access_token: session.access_token.clone(),
        xuid: session.xuid.clone(),
        client_id: crate::state::client_id(),
    };
    {
        let games = lock(&state.games);
        if games.contains_key(&id) {
            return Err(CmdError::new("busy", "this instance is already running"));
        }
    }
    let key = super::instances::install_key(&instance).ok_or(CmdError::new(
        "invalid_input",
        "this instance has an invalid loader configuration",
    ))?;
    if lock(&state.queue).is_busy_with(JobKind::Version, &key.to_key()) {
        return Err(CmdError::new(
            "busy",
            "this version is still being installed",
        ));
    }

    let cmd = prepare_launch(
        &state.layout,
        LaunchRequest {
            version_id: key.launch_id(),
            dir_name: instance.dir_name.clone(),
            auth,
            max_memory_mb: instance
                .max_memory_mb
                .unwrap_or(settings.default_max_memory_mb),
            extra_jvm_args: instance.jvm_args.clone(),
            java_override: instance.java_path.as_ref().map(std::path::PathBuf::from),
            game_dir: instance.game_dir.as_ref().map(std::path::PathBuf::from),
        },
    )
    .await?;

    // Batch log lines: at most one IPC message per 100 ms per game.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LogLine>();
    let (log_app, log_id) = (app.clone(), id.clone());
    tauri::async_runtime::spawn(async move {
        let mut batch = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let emit = |lines: Vec<LogLine>| {
            let _ = log_app.emit(
                "game://log",
                LogBatch {
                    instance_id: log_id.clone(),
                    lines,
                },
            );
        };
        loop {
            tokio::select! {
                line = rx.recv() => match line {
                    Some(l) => {
                        batch.push(l);
                        if batch.len() >= 500 { emit(std::mem::take(&mut batch)); }
                    }
                    None => { if !batch.is_empty() { emit(batch); } break; }
                },
                _ = tick.tick() => if !batch.is_empty() { emit(std::mem::take(&mut batch)); },
            }
        }
    });

    let mut game = process::spawn(
        &cmd,
        Arc::new(move |l| {
            let _ = tx.send(l);
        }),
    )?;
    let stop = CancellationToken::new();
    let started_at = now_unix();
    lock(&state.games).insert(
        id.clone(),
        GameHandle {
            stop: stop.clone(),
            started_at,
        },
    );
    tracing::info!(instance = %id, version = %instance.game_version, pid = ?game.pid, "game started");
    let _ = app.emit("game://started", &id);
    if settings.hide_on_launch
        && let Some(w) = app.get_webview_window("main")
    {
        let _ = w.hide();
    }

    let exit_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let code = tokio::select! {
            r = game.wait() => r.ok().flatten(),
            _ = stop.cancelled() => { let _ = game.kill().await; game.wait().await.ok().flatten() }
        };
        let seconds = now_unix() - started_at;
        if let Some(st) = exit_app.try_state::<AppState>() {
            lock(&st.games).remove(&id);
            if let Some(db) = st.store.clone() {
                let rid = id.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    db.record_play(&rid, started_at, seconds)
                })
                .await;
            }
        }
        if let Some(w) = exit_app.get_webview_window("main") {
            let _ = w.show();
        }
        tracing::info!(instance = %id, ?code, seconds, "game exited");
        let _ = exit_app.emit(
            "game://exit",
            ExitEvent {
                instance_id: id,
                code,
                seconds,
            },
        );
    });
    Ok(())
}

#[tauri::command]
pub fn stop_instance(state: State<'_, AppState>, id: String) -> Result<()> {
    validate_instance_id(&id)?;
    match lock(&state.games).get(&id) {
        Some(g) => {
            tracing::info!(instance = %id, "force stop requested");
            g.stop.cancel();
            Ok(())
        }
        None => Err(CmdError::new("not_running", "this instance is not running")),
    }
}
