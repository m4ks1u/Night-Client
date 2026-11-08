//! Tests against real Mojang / Fabric metadata stored in `tests/fixtures`.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use nc_core::{
    Secret, java,
    launch::{self, LaunchAuth, LaunchOptions, LaunchPaths},
    libraries::{self, LibraryKind},
    manifest::{VersionJson, VersionKind, VersionManifest},
    profile,
    rules::Environment,
};

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn version(id: &str) -> VersionJson {
    serde_json::from_str(&fixture(&format!("version-{id}.json"))).expect("parse version json")
}

fn paths() -> LaunchPaths {
    let root = PathBuf::from(r"C:\NC");
    LaunchPaths {
        java: root.join(r"runtimes\java\bin\javaw.exe"),
        game_dir: root.join(r"instances\poc"),
        assets_root: root.join("assets"),
        libraries_dir: root.join("libraries"),
        natives_dir: root.join(r"instances\poc\natives"),
        client_jar: root.join(r"versions\x\x.jar"),
        game_assets: root.join("assets"),
        log_config: Some(root.join(r"assets\log_configs\client.xml")),
    }
}

const TOKEN: &str = "eyJTOKEN-must-not-leak";

fn auth() -> LaunchAuth {
    LaunchAuth {
        player_name: "Steve".into(),
        uuid: "0123456789abcdef0123456789abcdef".into(),
        access_token: Secret::new(TOKEN),
        xuid: Some("2535000000000000".into()),
        client_id: Some("client-id".into()),
    }
}

fn opts() -> LaunchOptions {
    LaunchOptions {
        min_memory_mb: None,
        max_memory_mb: 4096,
        extra_jvm_args: vec![],
        resolution: None,
        launcher_name: "night-client".into(),
        launcher_version: "0.0.1".into(),
    }
}

#[test]
fn manifest_parses() {
    let m: VersionManifest = serde_json::from_str(&fixture("version_manifest_v2.json")).unwrap();
    assert_eq!(m.latest.release, "26.3");
    let e = m.find("1.8.9").unwrap();
    assert_eq!(e.kind, VersionKind::Release);
    assert_eq!(e.sha1.len(), 40);
    assert_eq!(m.find("b1.7.3").unwrap().kind, VersionKind::OldBeta);
}

#[test]
fn all_fixture_versions_parse_and_report_java() {
    assert_eq!(java::requirement(&version("26.3")).major_version, 25);
    assert_eq!(java::requirement(&version("1.21.1")).major_version, 21);
    assert_eq!(java::requirement(&version("1.16.5")).major_version, 8);
    let legacy = java::requirement(&version("1.8.9"));
    assert_eq!(
        (legacy.component.as_str(), legacy.major_version),
        ("jre-legacy", 8)
    );
}

#[test]
fn modern_windows_libraries_exclude_other_platforms() {
    let v = version("26.3");
    let libs = libraries::resolve(&v, &Environment::windows_x64()).unwrap();
    assert!(!libs.is_empty());
    assert!(libs.iter().all(|l| l.kind == LibraryKind::Classpath));
    assert!(
        libs.iter()
            .all(|l| !l.name.contains("natives-linux") && !l.name.contains("natives-macos"))
    );
    assert!(libs.iter().any(|l| l.name.ends_with(":natives-windows")));
    assert!(
        libs.iter()
            .all(|l| l.sha1.as_deref().is_some_and(|s| s.len() == 40))
    );
    assert!(
        libs.iter()
            .all(|l| l.url.as_deref().is_some_and(|u| u.starts_with("https://")))
    );
    // No duplicate classpath keys.
    let mut names: Vec<_> = libs.iter().map(|l| &l.path).collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len());
}

#[test]
fn legacy_natives_are_resolved_with_arch() {
    let v = version("1.8.9");
    let libs = libraries::resolve(&v, &Environment::windows_x64()).unwrap();
    let natives: Vec<_> = libs
        .iter()
        .filter(|l| matches!(l.kind, LibraryKind::Native { .. }))
        .collect();
    assert!(natives.iter().any(|l| {
        l.path
            .ends_with("lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar")
    }));
    assert!(natives.iter().any(|l| {
        l.path
            .ends_with("twitch-platform-6.5-natives-windows-64.jar")
    }));
    // The osx-only lwjgl 2.9.2 must not be present on Windows.
    assert!(!libs.iter().any(|l| l.name.contains("2.9.2-nightly")));
    let LibraryKind::Native { exclude } = &natives[0].kind else {
        unreachable!()
    };
    assert!(exclude.iter().any(|e| e == "META-INF/"));
}

#[test]
fn modern_launch_command_is_correct_and_redacted() {
    let v = version("26.3");
    let env = Environment::windows_x64();
    let libs = libraries::resolve(&v, &env).unwrap();
    let cmd = launch::build(&v, &env, &libs, &paths(), &auth(), &opts()).unwrap();
    let a = &cmd.args;

    assert_eq!(a[0], "-Xmx4096M");
    assert!(
        a.iter().any(|s| s.starts_with("-XX:HeapDumpPath=")),
        "windows-only arg present"
    );
    assert!(
        !a.iter().any(|s| s == "-XstartOnFirstThread"),
        "osx-only arg absent"
    );
    assert!(!a.iter().any(|s| s == "-Xss1M"), "x86-only arg absent");
    assert!(
        a.iter()
            .any(|s| s == r"-Djava.library.path=C:\NC\instances\poc\natives/java")
    );
    assert!(
        a.iter()
            .any(|s| s == r"-Dlog4j.configurationFile=C:\NC\assets\log_configs\client.xml")
    );

    let cp_idx = a.iter().position(|s| s == "-cp").unwrap();
    let cp = &a[cp_idx + 1];
    assert!(cp.ends_with(r"C:\NC\versions\x\x.jar"));
    assert!(cp.contains(';'));
    assert!(!cp.contains("${"));

    let main_idx = a
        .iter()
        .position(|s| s == "net.minecraft.client.main.Main")
        .unwrap();
    assert!(main_idx > cp_idx);
    let tok_idx = a.iter().position(|s| s == "--accessToken").unwrap();
    assert_eq!(a[tok_idx + 1], TOKEN);
    assert!(!a.iter().any(|s| s == "--demo"));
    assert!(!a.iter().any(|s| s == "--width"));
    assert!(
        !a.iter().any(|s| s.contains("${")),
        "unexpanded placeholder: {a:?}"
    );

    let red = cmd.redacted_args().join(" ");
    assert!(!red.contains(TOKEN));
    assert!(!format!("{cmd:?}").contains(TOKEN));
}

#[test]
fn resolution_feature_adds_width_and_height() {
    let v = version("1.21.1");
    let env = Environment::windows_x64();
    let libs = libraries::resolve(&v, &env).unwrap();
    let mut o = opts();
    o.resolution = Some((1280, 720));
    let cmd = launch::build(&v, &env, &libs, &paths(), &auth(), &o).unwrap();
    let w = cmd.args.iter().position(|s| s == "--width").unwrap();
    assert_eq!(cmd.args[w + 1], "1280");
    assert!(cmd.args.iter().any(|s| s == "--userType"));
}

#[test]
fn legacy_launch_command_uses_minecraft_arguments() {
    let v = version("1.8.9");
    let env = Environment::windows_x64();
    let libs = libraries::resolve(&v, &env).unwrap();
    let cmd = launch::build(&v, &env, &libs, &paths(), &auth(), &opts()).unwrap();
    let a = &cmd.args;
    assert!(
        a.iter()
            .any(|s| s == r"-Djava.library.path=C:\NC\instances\poc\natives")
    );
    let p = a.iter().position(|s| s == "--userProperties").unwrap();
    assert_eq!(a[p + 1], "{}");
    assert!(a.iter().any(|s| s == "--assetIndex") && a.iter().any(|s| s == "1.8"));
    // Natives jars are extracted, not put on the classpath.
    let cp = &a[a.iter().position(|s| s == "-cp").unwrap() + 1];
    assert!(!cp.contains("natives-windows"));
    assert!(!a.iter().any(|s| s.contains("${")));
}

#[test]
fn fabric_profile_merges_onto_vanilla() {
    let child: VersionJson = serde_json::from_str(&fixture("fabric-1.21.1.json")).unwrap();
    let merged = profile::merge(child, version("1.21.1")).unwrap();
    assert_eq!(
        merged.main_class.as_deref(),
        Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
    );
    assert!(merged.inherits_from.is_none());
    assert_eq!(java::requirement(&merged).major_version, 21);
    assert!(merged.asset_index.is_some());

    let env = Environment::windows_x64();
    let libs = libraries::resolve(&merged, &env).unwrap();
    let loader = libs
        .iter()
        .find(|l| l.name.starts_with("net.fabricmc:fabric-loader:"))
        .unwrap();
    assert!(
        loader
            .url
            .as_deref()
            .unwrap()
            .starts_with("https://maven.fabricmc.net/net/fabricmc/fabric-loader/")
    );
    // Finding: Fabric meta ships no hash for fabric-loader / intermediary, so the
    // installer must fetch the Maven `.sha1` sidecar for them (Stage 4).
    assert!(loader.sha1.is_none());
    let asm = libs
        .iter()
        .find(|l| l.name.starts_with("org.ow2.asm:asm:"))
        .unwrap();
    assert!(asm.sha1.is_some());

    let cmd = launch::build(&merged, &env, &libs, &paths(), &auth(), &opts()).unwrap();
    let main = cmd
        .args
        .iter()
        .position(|s| s.ends_with("KnotClient"))
        .unwrap();
    let fabric_jvm = cmd
        .args
        .iter()
        .position(|s| s.starts_with("-DFabricMcEmu="))
        .unwrap();
    assert!(
        fabric_jvm < main,
        "loader JVM args come before the main class"
    );
}

#[test]
fn merge_rejects_wrong_parent() {
    let child: VersionJson = serde_json::from_str(&fixture("fabric-1.21.1.json")).unwrap();
    assert!(profile::merge(child, version("26.3")).is_err());
}

#[test]
fn library_with_traversal_path_is_rejected() {
    let json = r#"{"id":"evil","mainClass":"x","libraries":[
        {"name":"a:b:1","downloads":{"artifact":{"path":"../../../Windows/evil.dll","sha1":"0000000000000000000000000000000000000000","size":1,"url":"https://example.com/x"}}}
    ]}"#;
    let v: VersionJson = serde_json::from_str(json).unwrap();
    assert!(libraries::resolve(&v, &Environment::windows_x64()).is_err());
}

#[test]
fn library_with_plain_http_url_is_rejected() {
    let json = r#"{"id":"evil","mainClass":"x","libraries":[
        {"name":"a:b:1","url":"http://insecure.example/"}
    ]}"#;
    let v: VersionJson = serde_json::from_str(json).unwrap();
    assert!(libraries::resolve(&v, &Environment::windows_x64()).is_err());
}
