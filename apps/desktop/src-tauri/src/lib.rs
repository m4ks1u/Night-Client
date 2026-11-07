//! Thin Tauri layer: IPC commands and events over `nc-launcher` services.
//!
//! Rules for this module:
//! * every command validates its input before touching the launcher;
//! * no token or secret is ever returned to the WebView;
//! * long work runs on the Tokio runtime, never on the main thread;
//! * high-frequency data (progress, logs) is throttled/batched before IPC.

mod commands;
mod state;

use std::{sync::OnceLock, time::Instant};

use tauri::Manager;

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

pub fn run() {
    STARTED.get_or_init(Instant::now);
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,night_client_lib=debug".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = state::AppState::new().map_err(|e| e.to_string())?;
            app.manage(state);
            create_main_window(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui_ready,
            commands::app_info,
            commands::list_versions,
            commands::install_version,
            commands::cancel_install,
            commands::account,
            commands::sign_in,
            commands::cancel_sign_in,
            commands::sign_out,
            commands::launch_game,
            commands::stop_game,
            commands::open_data_dir,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        tracing::error!(error = %e, "Night Client failed to start");
        std::process::exit(1);
    }
}
