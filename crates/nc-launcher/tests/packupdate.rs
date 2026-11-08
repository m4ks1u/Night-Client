//! Modpack updates end to end against a mock file server: an install of
//! version 1, user changes, then version 2 (and a failing version 2 that
//! must change nothing).

#![allow(clippy::unwrap_used)]

use std::{io::Write, path::Path};

use nc_core::{
    loader::LoaderKind,
    mrpack::{Pack, PackFile},
};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    modrinth::{modpack, packupdate},
};
use sha1::Digest as _;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn zip_with(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    let mut w = zip::ZipWriter::new(f.reopen().unwrap());
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap();
    f
}

fn file(server: &str, p: &str, data: &[u8]) -> PackFile {
    PackFile {
        path: p.into(),
        url: format!("{server}/{}", hex::encode(sha1::Sha1::digest(data))),
        sha1: hex::encode(sha1::Sha1::digest(data)),
        sha512: hex::encode(sha2::Sha512::digest(data)),
        size: data.len() as u64,
        optional: false,
    }
}

async fn serve(s: &MockServer, data: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/{}", hex::encode(sha1::Sha1::digest(data)))))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.to_vec()))
        .mount(s)
        .await;
}

fn pack(version: &str, files: Vec<PackFile>) -> Pack {
    Pack {
        name: "Test pack".into(),
        version_id: version.into(),
        summary: None,
        minecraft: "1.21.1".into(),
        loader: LoaderKind::Fabric,
        loader_version: Some("0.16.10".into()),
        files,
        skipped: 0,
    }
}

fn read(g: &Path, p: &str) -> Option<Vec<u8>> {
    std::fs::read(g.join(p)).ok()
}

#[tokio::test]
async fn update_replaces_untouched_files_and_keeps_user_changes() {
    let s = MockServer::start().await;
    for d in [&b"a1"[..], b"a2", b"b1", b"c1", b"n1"] {
        serve(&s, d).await;
    }
    let dl = Downloader::new("t", DownloaderConfig::default())
        .unwrap()
        .allow_insecure_for_tests();
    let c = CancellationToken::new();
    let g = tempfile::tempdir().unwrap();
    let g = g.path();

    // Version 1.
    let v1 = pack(
        "1",
        vec![
            file(&s.uri(), "mods/a.jar", b"a1"),
            file(&s.uri(), "mods/b.jar", b"b1"),
            file(&s.uri(), "mods/c.jar", b"c1"),
        ],
    );
    let z1 = zip_with(&[
        ("overrides/config/x.toml", b"x1"),
        ("overrides/options.txt", b"o1"),
    ]);
    modpack::download_files(&dl, g, &v1.files, &c, |_| {})
        .await
        .unwrap();
    let ov = modpack::extract_overrides(z1.path(), g, 1 << 20).unwrap();
    let tracked = modpack::tracked_after_install(&v1, &ov);
    assert_eq!(tracked.len(), 5);

    // The user edits options, disables c.jar and adds a mod of their own.
    std::fs::write(g.join("options.txt"), b"mine").unwrap();
    std::fs::rename(g.join("mods/c.jar"), g.join("mods/c.jar.disabled")).unwrap();
    std::fs::write(g.join("mods/own.jar"), b"own").unwrap();

    // Version 2: a.jar changes, b.jar is dropped, c.jar is unchanged, n.jar is new,
    // both overrides change.
    let v2 = pack(
        "2",
        vec![
            file(&s.uri(), "mods/a.jar", b"a2"),
            file(&s.uri(), "mods/c.jar", b"c1"),
            file(&s.uri(), "mods/n.jar", b"n1"),
        ],
    );
    let z2 = zip_with(&[
        ("overrides/config/x.toml", b"x2"),
        ("overrides/options.txt", b"o2"),
    ]);

    // A version 2 whose new file fails verification changes nothing.
    let mut broken = v2.clone();
    broken.files[2].sha512 = "0".repeat(128);
    assert!(
        packupdate::update(&dl, g, z2.path(), &broken, &tracked, "v2", &c, |_| {})
            .await
            .is_err()
    );
    assert_eq!(read(g, "mods/a.jar").unwrap(), b"a1");
    assert_eq!(read(g, "mods/b.jar").unwrap(), b"b1");
    assert_eq!(read(g, "config/x.toml").unwrap(), b"x1");
    assert!(!g.join(packupdate::STAGING).exists(), "staging cleaned up");

    let (r, pending) = packupdate::update(&dl, g, z2.path(), &v2, &tracked, "v2", &c, |_| {})
        .await
        .unwrap();
    // The old files are kept until the update is recorded.
    assert!(g.join(packupdate::STAGING).exists());
    packupdate::finish(pending).unwrap();
    assert_eq!(read(g, "mods/a.jar").unwrap(), b"a2", "untouched: replaced");
    assert!(
        read(g, "mods/b.jar").is_none(),
        "dropped and untouched: removed"
    );
    assert_eq!(read(g, "mods/n.jar").unwrap(), b"n1", "new: added");
    assert_eq!(read(g, "config/x.toml").unwrap(), b"x2");
    assert_eq!(read(g, "options.txt").unwrap(), b"mine", "edited: kept");
    assert_eq!(
        read(g, "mods/c.jar.disabled").unwrap(),
        b"c1",
        "disabled stays disabled"
    );
    assert!(read(g, "mods/c.jar").is_none());
    assert_eq!(
        read(g, "mods/own.jar").unwrap(),
        b"own",
        "the user's own file"
    );
    assert_eq!(
        (r.added, r.replaced, r.removed, r.unchanged, r.kept.clone()),
        (1, 2, 1, 1, vec!["options.txt".to_owned()])
    );
    assert!(!g.join(packupdate::STAGING).exists());

    // The edited file keeps its install-time hash, so it stays "the user's".
    let opt = r.tracked.iter().find(|t| t.path == "options.txt").unwrap();
    assert_eq!(opt.sha1, hex::encode(sha1::Sha1::digest(b"o1")));
    // Updating again to the same version changes nothing.
    let (again, pending) = packupdate::update(&dl, g, z2.path(), &v2, &r.tracked, "v2", &c, |_| {})
        .await
        .unwrap();
    packupdate::finish(pending).unwrap();
    assert_eq!(
        (again.added, again.replaced, again.removed),
        (0, 0, 0),
        "{again:?}"
    );
}
