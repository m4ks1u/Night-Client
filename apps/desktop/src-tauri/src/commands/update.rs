//! Launcher updates through the Tauri updater.
//!
//! The update public key and endpoint are compiled in from
//! `NIGHT_CLIENT_UPDATE_PUBKEY` and `NIGHT_CLIENT_UPDATE_ENDPOINT` by the
//! release workflow. Neither lives in the repository, and a build made
//! without them has no update channel at all (the plugin is not even
//! registered), so a development build can never trust a stray key.
//!
//! Every downloaded installer is verified against the public key before it
//! runs, and `requireSignedVersion` (tauri.conf.json) binds the signature to
//! the announced version, so an endpoint cannot replay an older release.

use std::{sync::atomic::Ordering, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, plugin::TauriPlugin};
use tauri_plugin_updater::{Update, UpdaterExt};

use super::{CmdError, Result, ensure_not_relocating};
use crate::state::{AppState, lock};

const PUBKEY: Option<&str> = option_env!("NIGHT_CLIENT_UPDATE_PUBKEY");
const ENDPOINT: Option<&str> = option_env!("NIGHT_CLIENT_UPDATE_ENDPOINT");

/// Release notes are shown as plain text; cap what we keep.
const MAX_NOTES: usize = 8_000;

fn channel() -> Option<(&'static str, &'static str)> {
    match (PUBKEY.map(str::trim), ENDPOINT.map(str::trim)) {
        (Some(k), Some(e)) if !k.is_empty() && !e.is_empty() => Some((k, e)),
        _ => None,
    }
}

/// The updater plugin, only for builds that carry an update channel.
pub fn plugin<R: Runtime>() -> Option<TauriPlugin<R, tauri_plugin_updater::Config>> {
    channel().map(|(key, _)| tauri_plugin_updater::Builder::new().pubkey(key).build())
}

/// The update found by the last check, kept until it is installed.
#[derive(Default)]
pub struct Pending(tokio::sync::Mutex<Option<Update>>);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// False for builds without an update channel (development builds).
    configured: bool,
    current_version: String,
    available: Option<AvailableUpdate>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    version: String,
    notes: String,
    date: Option<String>,
}

async fn check(app: &AppHandle) -> Result<UpdateInfo> {
    let mut info = UpdateInfo {
        configured: false,
        current_version: app.package_info().version.to_string(),
        available: None,
    };
    let Some((_, endpoint)) = channel() else {
        return Ok(info);
    };
    info.configured = true;
    let url = endpoint
        .parse()
        .map_err(|_| CmdError::new("internal", "the update address in this build is invalid"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .and_then(|b| b.timeout(Duration::from_secs(30)).build())
        .map_err(|e| CmdError::new("update", e.to_string()))?;
    let found = updater
        .check()
        .await
        .map_err(|e| CmdError::new("network", format!("could not check for updates: {e}")))?;
    if let Some(u) = &found {
        let mut notes = u.body.clone().unwrap_or_default();
        if notes.len() > MAX_NOTES {
            let cut = (0..=MAX_NOTES)
                .rev()
                .find(|&i| notes.is_char_boundary(i))
                .unwrap_or(0);
            notes.truncate(cut);
            notes.push('…');
        }
        info.available = Some(AvailableUpdate {
            version: u.version.clone(),
            notes,
            date: u.date.map(|d| d.to_string()),
        });
    }
    *app.state::<Pending>().0.lock().await = found;
    Ok(info)
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdateInfo> {
    let info = check(&app).await;
    if let Ok(i) = &info {
        tracing::info!(configured = i.configured, available = ?i.available.as_ref().map(|a| &a.version), "update check");
    }
    info
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    done_bytes: u64,
    total_bytes: Option<u64>,
}

/// Download the update found by the last check, verify its signature and
/// run the installer. On Windows the installer closes Night Client and
/// starts the new version, so on success this does not return.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
) -> Result<()> {
    ensure_not_relocating(&state)?;
    if !lock(&state.games).is_empty() {
        return Err(CmdError::new(
            "busy",
            "Close running games before updating Night Client.",
        ));
    }
    {
        let q = lock(&state.queue).snapshot();
        if q.active.is_some() || !q.pending.is_empty() {
            return Err(CmdError::new(
                "busy",
                "Wait for downloads to finish before updating Night Client.",
            ));
        }
    }
    let Some(update) = pending.0.lock().await.take() else {
        return Err(CmdError::new("not_found", "Check for updates first."));
    };
    // Block other work that writes data while the installer takes over.
    state.relocating.store(true, Ordering::Release);
    tracing::info!(version = %update.version, "downloading update");

    let emitter = app.clone();
    let mut done = 0u64;
    let mut last = 0u64;
    let bytes = update
        .download(
            move |chunk, total| {
                done += chunk as u64;
                // About 100 events at most.
                if total.is_none_or(|t| done - last >= (t / 100).max(1) || done >= t) {
                    last = done;
                    let _ = emitter.emit(
                        "update://progress",
                        Progress {
                            done_bytes: done,
                            total_bytes: total,
                        },
                    );
                }
            },
            || {},
        )
        .await;
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) => {
            state.relocating.store(false, Ordering::Release);
            tracing::warn!(error = %e, "update download or verification failed");
            return Err(CmdError::new(
                "update",
                format!("The update could not be downloaded or verified: {e}"),
            ));
        }
    };
    tracing::info!(version = %update.version, "update verified; starting installer");
    if let Err(e) = update.install(bytes) {
        state.relocating.store(false, Ordering::Release);
        return Err(CmdError::new(
            "update",
            format!("The installer could not start: {e}"),
        ));
    }
    // Non-Windows platforms (not shipped) would need a restart here.
    app.restart();
}

/// Background check shortly after startup, when enabled; tells the UI if a
/// newer version exists.
pub fn spawn_startup_check(app: AppHandle, enabled: bool) {
    if !enabled || channel().is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        match check(&app).await {
            Ok(UpdateInfo {
                available: Some(a), ..
            }) => {
                let _ = app.emit("update://available", a.version);
            }
            Ok(_) => {}
            Err(e) => tracing::info!(error = %e.message, "background update check failed"),
        }
    });
}
