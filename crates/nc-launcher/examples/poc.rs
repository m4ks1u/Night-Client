//! Stage 0 proof-of-concept CLI.
//!
//! ```text
//! cargo run --release -p nc-launcher --example poc -- install <version> [--data <dir>]
//! cargo run --release -p nc-launcher --example poc -- verify  <version> [--data <dir>]
//! NIGHT_CLIENT_MSA_CLIENT_ID=<guid> cargo run -p nc-launcher --example poc -- login
//! ```
//!
//! `verify` starts the installed client's main class with the real classpath
//! but *no* credentials. Minecraft refuses to start without `--accessToken`,
//! so this proves the runtime, classpath and main class resolve without
//! launching the game or bypassing authentication.

#![allow(clippy::unwrap_used)]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use nc_core::{launch, rules::Environment};
use nc_launcher::{
    CancellationToken,
    auth::{AuthConfig, Client},
    download::{Downloader, DownloaderConfig},
    install::{self, InstallProgress},
    layout::Layout,
    meta::Meta,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(Into::into)
        .unwrap_or_else(Layout::default_root);
    let layout = Layout::new(data);
    let dl = Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default())?;
    let meta = Meta::new(dl, layout.clone());
    let cancel = CancellationToken::new();
    {
        let c = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\ncancelling…");
            c.cancel();
        });
    }

    match (args.first().map(String::as_str), args.get(1)) {
        (Some("install"), Some(id)) => {
            let start = Instant::now();
            let last = Arc::new(std::sync::Mutex::new(Instant::now()));
            let progress = Arc::new(move |p: InstallProgress| {
                let mut l = last.lock().unwrap();
                if l.elapsed() > Duration::from_millis(500) {
                    *l = Instant::now();
                    eprintln!(
                        "{:?}: {}/{} files, {:.1}/{:.1} MiB",
                        p.phase,
                        p.progress.done_files,
                        p.progress.total_files,
                        p.progress.done_bytes as f64 / 1048576.0,
                        p.progress.total_bytes as f64 / 1048576.0
                    );
                }
            });
            let r =
                install::install_vanilla(&meta, id, &Environment::windows_x64(), &cancel, progress)
                    .await?;
            println!(
                "installed {id} in {:.1}s: game files downloaded={} skipped={} bytes={} | java={}",
                start.elapsed().as_secs_f64(),
                r.files.downloaded_files,
                r.files.skipped_files,
                r.files.downloaded_bytes,
                r.java.display()
            );
        }
        (Some("verify"), Some(id)) => {
            let version: nc_core::manifest::VersionJson =
                nc_launcher::meta::read_json(&layout.version_json(id)?, "version").await?;
            let req = nc_core::java::requirement(&version);
            let java = layout
                .runtime_dir(&req.component)?
                .join("bin")
                .join("java.exe");
            let libs = nc_core::libraries::resolve(&version, &Environment::windows_x64())?;
            let paths = launch::LaunchPaths {
                java: java.clone(),
                game_dir: layout.instance_dir("poc")?,
                assets_root: layout.assets_dir(),
                libraries_dir: layout.libraries_dir(),
                natives_dir: layout.natives_dir("poc", id)?,
                client_jar: layout.client_jar(id)?,
                log_config: None,
            };
            let cp = launch::classpath(&libs, &paths);
            let missing: Vec<_> = cp.iter().filter(|p| !p.is_file()).collect();
            assert!(missing.is_empty(), "missing classpath entries: {missing:?}");
            let cp = cp
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(";");
            let main = version.main_class.unwrap_or_default();
            let out = tokio::process::Command::new(&java)
                .args(["-cp", &cp, &main])
                .output()
                .await?;
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let text = format!("{stdout}{stderr}");
            let first: Vec<_> = text.lines().take(4).collect();
            println!("java exit={:?}\n{}", out.status.code(), first.join("\n"));
            if text.contains("ClassNotFoundException")
                || text.contains("NoClassDefFoundError")
                || text.contains("UnsatisfiedLinkError")
            {
                return Err("classpath verification FAILED".into());
            }
            println!(
                "verify: runtime + classpath + main class OK ({} classpath entries)",
                cp.split(';').count()
            );
        }
        (Some("login"), _) => {
            let id = std::env::var("NIGHT_CLIENT_MSA_CLIENT_ID")
                .map_err(|_| "set NIGHT_CLIENT_MSA_CLIENT_ID")?;
            let client = Client::new(AuthConfig::new(id), &nc_launcher::user_agent())?;
            let session = client
                .login_interactive(
                    |url| {
                        // Open the system browser without a shell.
                        std::process::Command::new("rundll32")
                            .args(["url.dll,FileProtocolHandler", url])
                            .spawn()
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    },
                    &cancel,
                    Duration::from_secs(300),
                )
                .await?;
            println!(
                "signed in as {} ({}) — token {:?}",
                session.profile.name, session.profile.id, session.access_token
            );
        }
        _ => eprintln!(
            "usage: poc install <version> | poc verify <version> | poc login  [--data <dir>]"
        ),
    }
    Ok(())
}
