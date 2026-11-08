//! Real-network version matrix. Ignored by default (downloads ~1–2 GB).
//!
//! ```text
//! cargo test -p nc-launcher --test matrix -- --ignored --nocapture
//! NC_MATRIX_DATA=<dir> to use a specific data directory (default: the real one, as a cache)
//! NC_MATRIX_VERSIONS=1.8.9,26.3 to limit the set
//! ```
//!
//! For each version: install, verify the install marker, prepare a launch
//! command (no process is started, so no authentication is bypassed), then
//! check that the main class is present in the classpath, legacy natives
//! were extracted, and legacy asset layouts exist.

#![allow(clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc, time::Instant};

use nc_core::{Secret, launch::LaunchAuth, rules::Environment};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    install,
    instance::{LaunchRequest, prepare_launch},
    layout::Layout,
    meta::Meta,
};

const DEFAULT: &[&str] = &[
    "1.5.2", "1.6.4", "1.8.9", "1.12.2", "1.16.5", "1.20.1", "1.21.1", "26.3",
];

fn class_in_jars(classpath: &str, class: &str) -> bool {
    let entry = format!("{}.class", class.replace('.', "/"));
    classpath.split(';').any(|jar| {
        std::fs::File::open(jar)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
            .is_some_and(|mut z| z.by_name(&entry).is_ok())
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads real game files"]
async fn version_matrix() {
    let root = std::env::var_os("NC_MATRIX_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(Layout::default_root);
    let versions: Vec<String> = std::env::var("NC_MATRIX_VERSIONS")
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|_| DEFAULT.iter().map(|s| s.to_string()).collect());
    let layout = Layout::new(&root);
    let meta = Meta::new(
        Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default()).unwrap(),
        layout.clone(),
    );
    let mut report = Vec::new();

    for id in &versions {
        let t = Instant::now();
        let installed = install::install_vanilla(
            &meta,
            id,
            &Environment::windows_x64(),
            &CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap_or_else(|e| panic!("{id}: install failed: {e}"));
        assert!(
            layout.installed_marker(id).unwrap().is_file(),
            "{id}: marker"
        );

        let dir = format!("matrix-{}", id.replace('.', "_"));
        let cmd = prepare_launch(
            &layout,
            LaunchRequest {
                version_id: id.clone(),
                dir_name: dir.clone(),
                auth: LaunchAuth {
                    player_name: "Matrix".into(),
                    uuid: "00000000000000000000000000000000".into(),
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
        .unwrap_or_else(|e| panic!("{id}: prepare_launch failed: {e}"));

        let args = &cmd.args;
        let cp_i = args
            .iter()
            .position(|a| a == "-cp")
            .unwrap_or_else(|| panic!("{id}: no -cp"));
        let main = installed.version.main_class.clone().unwrap();
        assert!(
            class_in_jars(&args[cp_i + 1], &main),
            "{id}: main class {main} not on classpath"
        );
        assert!(cmd.program.is_file(), "{id}: java missing");
        assert!(
            !args.iter().any(|a| a.contains("${")),
            "{id}: unexpanded placeholder in {:?}",
            cmd.redacted_args()
        );

        let natives = layout.natives_dir(&dir, id).unwrap();
        let has_legacy_natives = installed
            .libraries
            .iter()
            .any(|l| matches!(l.kind, nc_core::libraries::LibraryKind::Native { .. }));
        let dlls = std::fs::read_dir(&natives)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "dll"))
                    .count()
            })
            .unwrap_or(0);
        if has_legacy_natives {
            assert!(dlls > 0, "{id}: legacy natives not extracted");
        }

        let ai = installed.version.asset_index.as_ref().unwrap();
        let note = match ai.id.as_str() {
            "pre-1.6" => {
                let res = layout.instance_dir(&dir).unwrap().join("resources");
                assert!(
                    res.join("sound3").is_dir()
                        || res.join("music").is_dir()
                        || std::fs::read_dir(&res).unwrap().count() > 10,
                    "{id}: resources not laid out"
                );
                "map_to_resources"
            }
            "legacy" => {
                assert!(
                    layout
                        .assets_dir()
                        .join("virtual/legacy/sounds.json")
                        .is_file()
                        || layout.assets_dir().join("virtual/legacy/sound").is_dir(),
                    "{id}: virtual assets missing"
                );
                "virtual"
            }
            _ => "hashed",
        };
        report.push(format!(
            "{id:>7}  java {:>2}  libs {:>3}  natives dlls {:>2}  assets {note:<16} {:>5.1}s  main {main}",
            nc_core::java::requirement(&installed.version).major_version,
            installed.libraries.len(),
            dlls,
            t.elapsed().as_secs_f64(),
        ));
        let _ = std::fs::remove_dir_all(layout.instance_dir(&dir).unwrap());
    }
    println!("\n{}", report.join("\n"));
}
