use std::{sync::Arc, time::Duration};

use nc_core::{
    launch::{self, LaunchAuth, LaunchOptions, LaunchPaths},
    manifest::VersionKind,
    rules::Environment,
};
use nc_launcher::{
    CancellationToken, LauncherError,
    install::{self, InstallProgress},
    process::{self, LogLine},
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::state::{AppState, GameHandle, lock};

/// Error shape sent to the UI: a stable code plus a human message.
#[derive(Debug, Serialize)]
pub struct CmdError {
    code: &'static str,
    message: String,
}

impl From<LauncherError> for CmdError {
    fn from(e: LauncherError) -> Self {
        Self {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

impl From<nc_launcher::auth::AuthError> for CmdError {
    fn from(e: nc_launcher::auth::AuthError) -> Self {
        LauncherError::from(e).into()
    }
}

fn invalid(message: impl Into<String>) -> CmdError {
    CmdError {
        code: "invalid_input",
        message: message.into(),
    }
}

type Result<T> = std::result::Result<T, CmdError>;

/// Version ids come from the WebView: accept only the characters official ids use.
fn validate_version_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        && nc_core::paths::validate_segment(id).is_ok();
    if ok {
        Ok(())
    } else {
        Err(invalid("invalid version id"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    data_dir: String,
    sign_in_configured: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.meta.layout.root().display().to_string(),
        sign_in_configured: state.auth.is_some(),
    }
}

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
    versions: Vec<VersionItem>,
    from_cache: bool,
}

#[tauri::command]
pub async fn list_versions(state: State<'_, AppState>) -> Result<VersionList> {
    let (m, from_cache) = state
        .meta
        .version_manifest(&CancellationToken::new())
        .await?;
    let layout = &state.meta.layout;
    let versions = m
        .versions
        .into_iter()
        .filter(|v| v.kind == VersionKind::Release)
        .map(|v| VersionItem {
            installed: layout.installed_marker(&v.id).is_ok_and(|p| p.is_file()),
            id: v.id,
            kind: v.kind,
            release_time: v.release_time,
        })
        .collect();
    Ok(VersionList {
        latest_release: m.latest.release,
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
        if slot.is_some() {
            return Err(CmdError {
                code: "busy",
                message: "an installation is already running".into(),
            });
        }
        let t = CancellationToken::new();
        *slot = Some(t.clone());
        t
    };
    let progress_id = id.clone();
    let on_progress = Arc::new(move |progress: InstallProgress| {
        let _ = app.emit(
            "install://progress",
            ProgressEvent {
                id: progress_id.clone(),
                progress,
            },
        );
    });
    let r = install::install_vanilla(
        &state.meta,
        &id,
        &Environment::windows_x64(),
        &token,
        on_progress,
    )
    .await;
    *lock(&state.install) = None;
    let r = r?;
    Ok(InstallResult {
        downloaded_files: r.files.downloaded_files,
        skipped_files: r.files.skipped_files,
        downloaded_bytes: r.files.downloaded_bytes,
    })
}

#[tauri::command]
pub fn cancel_install(state: State<'_, AppState>) {
    if let Some(t) = lock(&state.install).as_ref() {
        t.cancel();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    name: String,
    uuid: String,
}

#[tauri::command]
pub fn account(state: State<'_, AppState>) -> Option<AccountView> {
    lock(&state.session).as_ref().map(|s| AccountView {
        name: s.profile.name.clone(),
        uuid: s.profile.id.clone(),
    })
}

#[tauri::command]
pub async fn sign_in(app: AppHandle, state: State<'_, AppState>) -> Result<AccountView> {
    let Some(client) = state.auth.as_ref() else {
        return Err(CmdError {
            code: "auth_not_configured",
            message:
                "Microsoft sign-in is not configured in this build (NIGHT_CLIENT_MSA_CLIENT_ID)."
                    .into(),
        });
    };
    let token = CancellationToken::new();
    *lock(&state.sign_in) = Some(token.clone());
    let opener = app.clone();
    let r = client
        .login_interactive(
            move |url| {
                opener
                    .opener()
                    .open_url(url, None::<&str>)
                    .map_err(|e| e.to_string())
            },
            &token,
            Duration::from_secs(300),
        )
        .await;
    *lock(&state.sign_in) = None;
    let session = r?;
    let view = AccountView {
        name: session.profile.name.clone(),
        uuid: session.profile.id.clone(),
    };
    tracing::info!(player = %view.name, "signed in");
    *lock(&state.session) = Some(session);
    Ok(view)
}

#[tauri::command]
pub fn cancel_sign_in(state: State<'_, AppState>) {
    if let Some(t) = lock(&state.sign_in).as_ref() {
        t.cancel();
    }
}

#[tauri::command]
pub fn sign_out(state: State<'_, AppState>) {
    *lock(&state.session) = None;
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitEvent {
    version: String,
    code: Option<i32>,
}

#[tauri::command]
pub async fn launch_game(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    max_memory_mb: u32,
) -> Result<()> {
    validate_version_id(&id)?;
    if !(512..=65536).contains(&max_memory_mb) {
        return Err(invalid("memory must be between 512 and 65536 MiB"));
    }
    let auth = {
        let session = lock(&state.session);
        let s = session.as_ref().ok_or(CmdError {
            code: "auth_required",
            message: "Sign in with Microsoft to play.".into(),
        })?;
        LaunchAuth {
            player_name: s.profile.name.clone(),
            uuid: s.profile.id.clone(),
            access_token: s.access_token.clone(),
            xuid: s.xuid.clone(),
            client_id: crate::state::client_id(),
        }
    };
    if lock(&state.game).is_some() {
        return Err(CmdError {
            code: "busy",
            message: "the game is already running".into(),
        });
    }

    let layout = state.meta.layout.clone();
    if !layout
        .installed_marker(&id)
        .map_err(LauncherError::from)?
        .is_file()
    {
        return Err(LauncherError::NotInstalled(id).into());
    }
    let version: nc_core::manifest::VersionJson = nc_launcher::meta::read_json(
        &layout.version_json(&id).map_err(LauncherError::from)?,
        "version",
    )
    .await?;
    let env = Environment::windows_x64();
    let libs = nc_core::libraries::resolve(&version, &env).map_err(LauncherError::from)?;
    let req = nc_core::java::requirement(&version);
    let instance = "default";
    let natives = layout
        .natives_dir(instance, &id)
        .map_err(LauncherError::from)?;
    install::prepare_natives(layout.libraries_dir(), libs.clone(), natives.clone()).await?;

    let paths = LaunchPaths {
        java: layout
            .runtime_dir(&req.component)
            .map_err(LauncherError::from)?
            .join("bin")
            .join("javaw.exe"),
        game_dir: layout.instance_dir(instance).map_err(LauncherError::from)?,
        assets_root: layout.assets_dir(),
        libraries_dir: layout.libraries_dir(),
        natives_dir: natives,
        client_jar: layout.client_jar(&id).map_err(LauncherError::from)?,
        log_config: version
            .logging
            .as_ref()
            .and_then(|l| l.client.as_ref())
            .and_then(|c| layout.log_config(&c.file.id).ok()),
    };
    let opts = LaunchOptions {
        min_memory_mb: None,
        max_memory_mb,
        extra_jvm_args: vec![],
        resolution: None,
        launcher_name: "night-client".into(),
        launcher_version: env!("CARGO_PKG_VERSION").into(),
    };
    let cmd =
        launch::build(&version, &env, &libs, &paths, &auth, &opts).map_err(LauncherError::from)?;

    // Batch log lines: at most one IPC message per 100 ms.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LogLine>();
    let log_app = app.clone();
    tokio::spawn(async move {
        let mut batch = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                line = rx.recv() => match line {
                    Some(l) => { batch.push(l); if batch.len() >= 500 { let _ = log_app.emit("game://log", std::mem::take(&mut batch)); } }
                    None => { if !batch.is_empty() { let _ = log_app.emit("game://log", batch); } break; }
                },
                _ = tick.tick() => if !batch.is_empty() { let _ = log_app.emit("game://log", std::mem::take(&mut batch)); },
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
    *lock(&state.game) = Some(GameHandle {
        stop: stop.clone(),
        version: id.clone(),
    });

    let exit_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let code = tokio::select! {
            r = game.wait() => r.ok().flatten(),
            _ = stop.cancelled() => { let _ = game.kill().await; game.wait().await.ok().flatten() }
        };
        if let Some(st) = exit_app.try_state::<AppState>() {
            *lock(&st.game) = None;
        }
        tracing::info!(?code, "game exited");
        let _ = exit_app.emit("game://exit", ExitEvent { version: id, code });
    });
    Ok(())
}

use tauri::Manager as _;

#[tauri::command]
pub fn stop_game(state: State<'_, AppState>) {
    if let Some(g) = lock(&state.game).as_ref() {
        tracing::info!(version = %g.version, "stopping game on user request");
        g.stop.cancel();
    }
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let dir = state.meta.layout.root().to_path_buf();
    std::fs::create_dir_all(&dir).map_err(LauncherError::io(&dir))?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| CmdError {
            code: "io",
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::validate_version_id;

    #[test]
    fn version_id_validation() {
        for ok in ["1.21.1", "26.3", "26.4-snapshot-7", "1.14 Pre-Release 1"] {
            assert!(validate_version_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:x",
            "x;calc",
            "con",
            &"x".repeat(65),
        ] {
            assert!(validate_version_id(bad).is_err(), "{bad}");
        }
    }
}
