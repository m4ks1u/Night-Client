use nc_launcher::{logging, sys};
use nc_store::AppSettings;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::{CmdError, Result, with_db};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    data_dir: String,
    log_dir: String,
    sign_in_configured: bool,
    startup_error: Option<String>,
    gpu_active: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.layout.root().display().to_string(),
        log_dir: state.log_dir.display().to_string(),
        sign_in_configured: state.auth.is_some(),
        startup_error: state.startup_error.clone(),
        gpu_active: state.gpu_active,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemView {
    #[serde(flatten)]
    info: sys::SystemInfo,
    recommended_memory_mb: u32,
    max_memory_mb: u32,
}

#[tauri::command]
pub fn system_info() -> SystemView {
    let info = sys::system_info();
    SystemView {
        recommended_memory_mb: sys::recommended_memory_mb(info.total_memory_mb),
        max_memory_mb: sys::max_memory_mb(info.total_memory_mb),
        info,
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings> {
    with_db(&state, |db| Ok(db.app_settings())).await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings> {
    let s = settings.clone();
    with_db(&state, move |db| db.save_app_settings(&s)).await?;
    state.apply_settings(&settings);
    tracing::info!(?settings, "settings updated");
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeView {
    component: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    app_version: &'static str,
    os: String,
    total_memory_mb: u64,
    cpu_threads: usize,
    data_dir: String,
    log_dir: String,
    log_file: Option<String>,
    database: Option<String>,
    schema_version: Option<u32>,
    startup_error: Option<String>,
    instance_count: Option<usize>,
    installed_versions: Vec<String>,
    runtimes: Vec<RuntimeView>,
    sign_in_configured: bool,
}

#[tauri::command]
pub async fn diagnostics(state: State<'_, AppState>) -> Result<Diagnostics> {
    let layout = state.layout.clone();
    let (installed_versions, runtimes) = tauri::async_runtime::spawn_blocking(move || {
        let mut versions = Vec::new();
        if let Ok(rd) = std::fs::read_dir(layout.root().join("versions")) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if layout.installed_marker(&id).is_ok_and(|m| m.is_file()) {
                    versions.push(id);
                }
            }
        }
        let mut runtimes = Vec::new();
        if let Ok(rd) = std::fs::read_dir(layout.runtimes_dir()) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                if let Ok(v) = std::fs::read_to_string(e.path().join(".version")) {
                    runtimes.push(RuntimeView {
                        component: e.file_name().to_string_lossy().into_owned(),
                        version: v,
                    });
                }
            }
        }
        versions.sort();
        (versions, runtimes)
    })
    .await
    .map_err(|e| CmdError::new("internal", e.to_string()))?;

    let info = sys::system_info();
    let db = state.store.clone();
    let (schema_version, instance_count) = match &db {
        Some(db) => (
            db.schema_version().ok(),
            db.list_instances().ok().map(|l| l.len()),
        ),
        None => (None, None),
    };
    Ok(Diagnostics {
        app_version: env!("CARGO_PKG_VERSION"),
        os: info.os_version,
        total_memory_mb: info.total_memory_mb,
        cpu_threads: info.cpu_threads,
        data_dir: state.layout.root().display().to_string(),
        log_dir: state.log_dir.display().to_string(),
        log_file: logging::current_log_file(&state.log_dir).map(|p| p.display().to_string()),
        database: db
            .as_ref()
            .and_then(|d| d.path())
            .map(|p| p.display().to_string()),
        schema_version,
        startup_error: state.startup_error.clone(),
        instance_count,
        installed_versions,
        runtimes,
        sign_in_configured: state.auth.is_some(),
    })
}

#[tauri::command]
pub async fn recent_logs(state: State<'_, AppState>, lines: u32) -> Result<Vec<String>> {
    let lines = lines.clamp(1, 2000) as usize;
    let dir = state.log_dir.clone();
    tauri::async_runtime::spawn_blocking(move || match logging::current_log_file(&dir) {
        Some(f) => logging::tail(&f, lines).map_err(|e| CmdError::new("io", e.to_string())),
        None => Ok(Vec::new()),
    })
    .await
    .map_err(|e| CmdError::new("internal", e.to_string()))?
}

fn open(app: &AppHandle, path: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| CmdError::new("io", e.to_string()))?;
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| CmdError::new("io", e.to_string()))
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    open(&app, state.layout.root())
}

#[tauri::command]
pub fn open_logs_dir(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    open(&app, &state.log_dir)
}

pub(crate) fn open_dir(app: &AppHandle, path: &std::path::Path) -> Result<()> {
    open(app, path)
}
