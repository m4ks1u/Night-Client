//! Real-network loader installs (Fabric, NeoForge). Ignored by default.
//!
//! ```text
//! cargo test --release -p nc-launcher --test loaders -- --ignored --nocapture
//! ```
//!
//! Installs the newest stable loader for each case, prepares the launch
//! command (nothing is started), and checks: the main class is present in a
//! classpath or module-path jar, the per-version client jar exists, NeoForge
//! produced its patched client, and no placeholder is left unexpanded.

#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Instant};

use nc_core::{
    Secret,
    launch::LaunchAuth,
    loader::{InstallKey, LoaderKind},
    rules::Environment,
};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    instance::{LaunchRequest, prepare_launch},
    layout::Layout,
    loaders,
    meta::Meta,
};

fn jar_has(jar: &str, entry: &str) -> bool {
    std::fs::File::open(jar)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|mut z| z.by_name(entry).is_ok())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads real loader files and runs NeoForge processors"]
async fn loader_matrix() {
    let root = std::env::var_os("NC_MATRIX_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(Layout::default_root);
    let layout = Layout::new(&root);
    let meta = Meta::new(
        Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default()).unwrap(),
        layout.clone(),
    );
    let cancel = CancellationToken::new();
    let mut report = Vec::new();

    for (kind, mc) in [
        (LoaderKind::Fabric, "1.21.1"),
        (LoaderKind::Fabric, "26.3"),
        (LoaderKind::NeoForge, "1.21.1"),
        (LoaderKind::NeoForge, "26.3"),
    ] {
        let versions = loaders::versions(&meta, kind, mc, &cancel).await.unwrap();
        let Some(lv) = versions.iter().find(|v| v.stable).or(versions.first()) else {
            report.push(format!(
                "{kind:?} {mc}: no loader versions published, skipped"
            ));
            continue;
        };
        let key = InstallKey::new(kind, mc, Some(&lv.version)).unwrap();
        let t = Instant::now();
        let installed = loaders::install(
            &meta,
            &key,
            &Environment::windows_x64(),
            &cancel,
            Arc::new(|_| {}),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", key.to_key()));
        let secs = t.elapsed().as_secs_f64();
        let id = key.launch_id();
        assert!(layout.installed_marker(&id).unwrap().is_file());
        assert!(
            layout.client_jar(&id).unwrap().is_file(),
            "per-version client jar"
        );

        let dir = format!("loader-matrix-{}", id.replace('.', "_"));
        let cmd = prepare_launch(
            &layout,
            LaunchRequest {
                version_id: id.clone(),
                dir_name: dir.clone(),
                auth: LaunchAuth {
                    player_name: "Matrix".into(),
                    uuid: "0".repeat(32),
                    access_token: Secret::new("not-a-real-token"),
                    xuid: None,
                    client_id: None,
                },
                max_memory_mb: 2048,
                extra_jvm_args: vec![],
                java_override: None,
                game_dir: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{id}: prepare_launch: {e}"));

        let main = installed.version.main_class.clone().unwrap();
        let entry = format!("{}.class", main.replace('.', "/"));
        let a = &cmd.args;
        let mut jars: Vec<String> = Vec::new();
        for flag in ["-cp", "-p"] {
            if let Some(i) = a.iter().position(|x| x == flag) {
                jars.extend(a[i + 1].split(';').map(str::to_owned));
            }
        }
        assert!(
            jars.iter().any(|j| jar_has(j, &entry)),
            "{id}: main class {main} not found in classpath/module path"
        );
        assert!(
            !a.iter().any(|x| x.contains("${")),
            "{id}: unexpanded placeholder"
        );
        let missing: Vec<_> = jars
            .iter()
            .filter(|j| !std::path::Path::new(j).is_file())
            .collect();
        assert!(missing.is_empty(), "{id}: missing jars {missing:?}");

        let mut note = String::new();
        if kind == LoaderKind::NeoForge {
            // 20.x/21.x: neoforge-<v>-client.jar; 26.x: minecraft-client-patched-<v>.jar.
            let candidates = [
                format!(
                    "net/neoforged/neoforge/{0}/neoforge-{0}-client.jar",
                    lv.version
                ),
                format!(
                    "net/neoforged/minecraft-client-patched/{0}/minecraft-client-patched-{0}.jar",
                    lv.version
                ),
            ];
            let patched = candidates
                .iter()
                .map(|c| layout.libraries_dir().join(c))
                .find(|p| p.is_file())
                .unwrap_or_else(|| panic!("{id}: patched client missing"));
            note = format!(
                "patched client {:.1} MB",
                std::fs::metadata(&patched).unwrap().len() as f64 / 1048576.0
            );
        }
        report.push(format!(
            "{:<34} {:>6.1}s  libs {:>3}  main {main}  {note}",
            key.label(),
            secs,
            installed.libraries.len()
        ));
        let _ = std::fs::remove_dir_all(layout.instance_dir(&dir).unwrap());
    }
    println!("\n{}", report.join("\n"));
}
