//! Thin Tauri layer: IPC commands and events over `nc-launcher` and
//! `nc-store` services.
//!
//! Rules for this crate:
//! * every command validates its input before touching a service;
//! * no token or secret is ever returned to the WebView;
//! * long work runs on the async runtime or the blocking pool, never on the
//!   main thread;
//! * high-frequency data (progress, logs) is throttled/batched before IPC.

mod commands;
mod state;

use std::{path::PathBuf, sync::OnceLock, time::Instant};

use nc_launcher::layout::Layout;
use tauri::Manager;

/// The full list of IPC commands. `build.rs` generates one `allow-*`
/// permission per entry; `capabilities/main.json` grants them to the main
/// window only.
macro_rules! app_commands {
    () => {
        tauri::generate_handler![
            ui_ready,
            commands::app::app_info,
            commands::app::system_info,
            commands::app::get_settings,
            commands::app::update_settings,
            commands::app::diagnostics,
            commands::app::recent_logs,
            commands::app::open_data_dir,
            commands::app::open_logs_dir,
            commands::versions::list_versions,
            commands::downloads::install_version,
            commands::downloads::cancel_download,
            commands::downloads::download_queue,
            commands::downloads::download_history,
            commands::downloads::clear_download_history,
            commands::java::detect_java,
            commands::java::set_instance_java,
            commands::instances::list_instances,
            commands::instances::get_instance,
            commands::instances::create_instance,
            commands::instances::update_instance,
            commands::instances::delete_instance,
            commands::instances::open_instance_dir,
            commands::instances::set_instance_jvm_args,
            commands::instances::loader_versions,
            commands::instances::set_instance_loader_version,
            commands::instances::set_instance_game_dir,
            commands::instances::pick_folder,
            commands::mods::modrinth_search,
            commands::mods::modrinth_project,
            commands::mods::modrinth_categories,
            commands::mods::open_modrinth_page,
            commands::mods::mod_install_plan,
            commands::mods::mod_install,
            commands::mods::list_mods,
            commands::mods::set_mod_enabled,
            commands::mods::remove_mod,
            commands::mods::check_mod_updates,
            commands::mods::update_mods,
            commands::mods::import_local_mod,
            commands::account::account,
            commands::account::sign_in,
            commands::account::cancel_sign_in,
            commands::account::list_accounts,
            commands::account::check_active_account,
            commands::account::switch_account,
            commands::account::remove_account,
            commands::game::running_instances,
            commands::game::launch_instance,
            commands::game::stop_instance,
        ]
    };
}

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Called once by the UI after its first committed render; records
/// process-start → UI-ready time for the startup benchmark.
#[tauri::command]
fn ui_ready() -> u128 {
    let ms = STARTED.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
    tracing::info!(target: "bench", ui_ready_ms = ms, "ui ready");
    ms
}

/// The main window is built in code (config has `create: false`) so debug
/// builds can expose the WebView2 DevTools port for automated smoke tests.
/// Release builds never open a debugging port: any local process could
/// otherwise drive the UI.
fn create_main_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = app
        .config()
        .app
        .windows
        .first()
        .ok_or("missing main window config")?
        .clone();
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?;
    #[cfg(debug_assertions)]
    if let Some(port) = std::env::var("NIGHT_CLIENT_DEVTOOLS_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        tracing::warn!(port, "DEBUG BUILD: WebView2 remote debugging enabled");
        builder = builder.additional_browser_args(&format!(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
        ));
    }
    builder.build()?;
    Ok(())
}

/// Logs are machine-local: `%LOCALAPPDATA%\NightClient\logs`.
fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(Layout::default_root)
        .join("NightClient")
        .join("logs")
}

pub fn run() {
    STARTED.get_or_init(Instant::now);
    let log_dir = log_dir();
    // Keep the guard alive for the whole process so buffered lines are flushed.
    let _log_guard =
        match nc_launcher::logging::init(&log_dir, "info,night_client_lib=debug,bench=info") {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("file logging unavailable: {e}");
                None
            }
        };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Night Client starting");

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            app.manage(state::AppState::new(
                Layout::new(Layout::default_root()),
                log_dir.clone(),
            ));
            commands::downloads::spawn_worker(app.handle().clone());
            // Remove temp files left by crashed downloads (older than a day,
            // so a recent interruption can still resume).
            let root = Layout::default_root();
            tauri::async_runtime::spawn_blocking(move || {
                let (n, bytes) =
                    nc_launcher::fsutil::sweep_stale(&root, std::time::Duration::from_secs(86_400));
                if n > 0 {
                    tracing::info!(files = n, bytes, "removed stale temporary files");
                }
            });
            create_main_window(app)?;
            Ok(())
        })
        .invoke_handler(app_commands!())
        .run(tauri::generate_context!());

    if let Err(e) = result {
        tracing::error!(error = %e, "Night Client failed to start");
        std::process::exit(1);
    }
}
