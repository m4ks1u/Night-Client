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
mod memory;
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
            commands::window::show_snap_layouts,
            commands::data::data_folder_info,
            commands::data::relocate_data,
            commands::update::check_for_update,
            commands::update::install_update,
            commands::app::take_crash_notice,
            commands::app::export_diagnostics,
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
    // Tauri's defaults, plus software rendering unless the user turned the
    // GPU on: the GPU process costs ~70 MB and this UI draws just as
    // smoothly without it (docs/benchmarks).
    let gpu = app.state::<state::AppState>().gpu_active;
    #[allow(unused_mut)]
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if !gpu {
        args.push_str(" --disable-gpu");
    }
    #[cfg(debug_assertions)]
    if let Some(port) = std::env::var("NIGHT_CLIENT_DEVTOOLS_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        tracing::warn!(port, "DEBUG BUILD: WebView2 remote debugging enabled");
        args.push_str(&format!(" --remote-debugging-port={port}"));
    }
    tracing::info!(gpu, "creating main window");
    tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
        .additional_browser_args(&args)
        .build()?;
    Ok(())
}

/// On a panic, log it and write a redacted crash report next to the logs
/// (shown to the user on the next start). Nothing is sent anywhere.
fn install_crash_reporter(log_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let details = format!(
            "{info}\nthread: {}\nos: {}",
            thread.name().unwrap_or("<unnamed>"),
            nc_launcher::sys::system_info().os_version
        );
        tracing::error!(panic = %nc_launcher::logging::redact(&info.to_string()), "Night Client crashed");
        if let Err(e) =
            nc_launcher::report::write_crash(&log_dir, env!("CARGO_PKG_VERSION"), &details)
        {
            tracing::error!(error = %e, "could not write crash report");
        }
        previous(info);
    }));
}

/// Logs are machine-local: `%LOCALAPPDATA%\NightClient\logs`.
fn log_dir() -> PathBuf {
    nc_launcher::layout::local_dir()
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
    install_crash_reporter(log_dir.clone());

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                memory::set_low(&w, false);
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());
    // Only builds with an update channel get the updater (see commands::update).
    if let Some(updater) = commands::update::plugin() {
        builder = builder.plugin(updater);
    }
    let result = builder
        .manage(commands::update::Pending::default())
        .on_window_event(memory::on_window_event)
        .setup(move |app| {
            let home = Layout::default_root();
            let root = nc_launcher::relocate::resolve_root(&home);
            tracing::info!(data = %root.display(), "data folder");
            app.manage(state::AppState::new(
                home,
                Layout::new(root.clone()),
                log_dir.clone(),
            ));
            commands::downloads::spawn_worker(app.handle().clone());
            let check_updates = app
                .state::<state::AppState>()
                .store
                .as_ref()
                .is_some_and(|s| s.app_settings().check_updates);
            commands::update::spawn_startup_check(app.handle().clone(), check_updates);
            // Debug builds only: e2e/crash-report.mjs uses this to exercise the
            // crash reporter (a panic on a side thread; the app keeps running).
            #[cfg(debug_assertions)]
            if std::env::var_os("NIGHT_CLIENT_TEST_PANIC").is_some() {
                std::thread::Builder::new()
                    .name("test-panic".into())
                    .spawn(|| {
                        // After the UI has made its startup crash check.
                        std::thread::sleep(std::time::Duration::from_secs(4));
                        panic!("test panic with access_token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.c2lnbmF0dXJlLXRlc3Q")
                    })?;
            }
            // Remove temp files left by crashed downloads (older than a day,
            // so a recent interruption can still resume).
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
