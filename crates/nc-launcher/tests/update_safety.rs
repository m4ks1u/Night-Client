//! Regression tests for the independent audit's findings (NC-AUDIT-001 to
//! 003), through the public API. The audit's characterization tests passed
//! while the defects reproduced; these assert the fixed behavior.

#![allow(clippy::unwrap_used)]

use std::io::Write;

use nc_core::{loader::LoaderKind, mrpack::Pack};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    fsutil,
    modrinth::{
        modpack::{self, Tracked},
        packupdate,
    },
};
use sha1::Digest as _;

fn archive(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    let mut z = zip::ZipWriter::new(f.reopen().unwrap());
    for (name, bytes) in entries {
        z.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap();
    f
}

fn pack() -> Pack {
    Pack {
        name: "Fixture".into(),
        version_id: "2".into(),
        summary: None,
        minecraft: "1.21.1".into(),
        loader: LoaderKind::Vanilla,
        loader_version: None,
        files: vec![],
        skipped: 0,
    }
}

fn dl() -> Downloader {
    Downloader::new("test", DownloaderConfig::default()).unwrap()
}

/// NC-AUDIT-002: a later destination that can't be written must leave the
/// earlier ones as they were, and nothing behind.
#[tokio::test]
async fn a_failed_update_leaves_the_instance_unchanged() {
    let g = tempfile::tempdir().unwrap();
    std::fs::write(g.path().join("a.txt"), b"old").unwrap();
    std::fs::write(g.path().join("blocker"), b"a file, not a directory").unwrap();
    let old = vec![Tracked {
        path: "a.txt".into(),
        sha1: hex::encode(sha1::Sha1::digest(b"old")),
        is_override: true,
    }];
    let z = archive(&[
        ("overrides/a.txt", b"new"),
        ("overrides/blocker/b.txt", b"b"),
    ]);
    let r = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &old,
        "v2",
        &CancellationToken::new(),
        |_| {},
    )
    .await;
    assert!(r.is_err(), "the blocked destination must fail");
    assert_eq!(std::fs::read(g.path().join("a.txt")).unwrap(), b"old");
    assert_eq!(
        std::fs::read(g.path().join("blocker")).unwrap(),
        b"a file, not a directory"
    );
    assert!(!g.path().join(packupdate::STAGING).exists());
    assert_eq!(
        packupdate::recover(g.path(), None).unwrap(),
        packupdate::Recovered::default()
    );
}

/// NC-AUDIT-003: whatever was already in `.nc-update` survives an update.
#[tokio::test]
async fn an_update_keeps_existing_content_in_the_staging_folder() {
    let g = tempfile::tempdir().unwrap();
    let staging = g.path().join(packupdate::STAGING);
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("user.txt"), b"user content").unwrap();
    let z = archive(&[("overrides/config/x.toml", b"x")]);
    let (_, pending) = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &[],
        "v2",
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    packupdate::finish(pending).unwrap();
    assert_eq!(
        std::fs::read(staging.join("user.txt")).unwrap(),
        b"user content"
    );
    assert_eq!(std::fs::read(g.path().join("config/x.toml")).unwrap(), b"x");
    // Recovery doesn't touch it either.
    packupdate::recover(g.path(), Some("v2")).unwrap();
    assert!(staging.join("user.txt").exists());
}

/// NC-AUDIT-003: a pack can't write into the launcher's staging folder.
#[test]
fn packs_cannot_write_into_the_staging_folder() {
    for evil in [
        "overrides/.nc-update/x",
        "client-overrides/.NC-UPDATE/journal.json",
    ] {
        let z = archive(&[(evil, b"x")]);
        let g = tempfile::tempdir().unwrap();
        assert!(
            modpack::extract_overrides(z.path(), g.path(), 1 << 20).is_err(),
            "{evil}"
        );
        assert!(!g.path().join(".nc-update").exists());
    }
}

/// A cancelled update changes nothing and leaves nothing behind.
#[tokio::test]
async fn a_cancelled_update_changes_nothing() {
    let g = tempfile::tempdir().unwrap();
    std::fs::write(g.path().join("a.txt"), b"old").unwrap();
    let old = vec![Tracked {
        path: "a.txt".into(),
        sha1: hex::encode(sha1::Sha1::digest(b"old")),
        is_override: true,
    }];
    let z = archive(&[("overrides/a.txt", b"new")]);
    let c = CancellationToken::new();
    c.cancel();
    let r = packupdate::update(&dl(), g.path(), z.path(), &pack(), &old, "v2", &c, |_| {}).await;
    assert_eq!(
        r.as_ref().map(|_| ()).unwrap_err().code(),
        "cancelled",
        "{r:?}"
    );
    assert_eq!(std::fs::read(g.path().join("a.txt")).unwrap(), b"old");
    assert!(!g.path().join(packupdate::STAGING).exists());
}

#[cfg(windows)]
fn junction(link: &std::path::Path, target: &std::path::Path) {
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// NC-AUDIT-001: extraction never writes through an existing junction.
#[cfg(windows)]
#[test]
fn extraction_does_not_follow_an_existing_junction() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
    junction(&root.path().join("linked"), outside.path());
    for entry in [
        "linked/probe.txt",
        "linked/sentinel",
        "linked/sub/deeper.txt",
    ] {
        let z = archive(&[(entry, b"outside write")]);
        assert!(
            fsutil::extract_zip(z.path(), root.path(), &[], 1024).is_err(),
            "{entry}"
        );
    }
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(outside.path().join("sentinel")).unwrap(),
        b"keep"
    );
    std::fs::remove_dir(root.path().join("linked")).unwrap();
}

/// NC-AUDIT-001: modpack overrides and updates don't follow junctions in
/// the instance either.
#[cfg(windows)]
#[tokio::test]
async fn modpack_writes_do_not_follow_junctions() {
    let g = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
    junction(&g.path().join("config"), outside.path());
    let z = archive(&[("overrides/config/x.toml", b"x")]);
    assert!(modpack::extract_overrides(z.path(), g.path(), 1 << 20).is_err());
    let r = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &[],
        "v2",
        &CancellationToken::new(),
        |_| {},
    )
    .await;
    assert!(r.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
    assert!(!g.path().join(packupdate::STAGING).exists());
    std::fs::remove_dir(g.path().join("config")).unwrap();
}

/// NC-AUDIT-001: a junction at the staging folder is refused, not used.
#[cfg(windows)]
#[tokio::test]
async fn a_junction_at_the_staging_folder_is_not_used() {
    let g = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    junction(&g.path().join(packupdate::STAGING), outside.path());
    let z = archive(&[("overrides/a.txt", b"x")]);
    let r = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &[],
        "v2",
        &CancellationToken::new(),
        |_| {},
    )
    .await;
    assert!(r.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(!g.path().join("a.txt").exists());
    std::fs::remove_dir(g.path().join(packupdate::STAGING)).unwrap();
}
