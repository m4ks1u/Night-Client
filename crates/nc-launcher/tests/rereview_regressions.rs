//! Regression tests for the re-review's findings (RR-002 to RR-004), through
//! the public API. The re-review's characterization tests passed while the
//! defects reproduced; these are the same scenarios asserting the fixed
//! behavior. RR-001 (atomic recording) is covered in
//! `nc-store/tests/update_record.rs`; real crashes in `packupdate`'s unit tests.

#![allow(clippy::unwrap_used)]

use std::io::Write;

use nc_core::{loader::LoaderKind, mrpack::Pack};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    modrinth::{modpack::Tracked, packupdate},
};
use sha1::Digest as _;

fn archive() -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    let mut z = zip::ZipWriter::new(f.reopen().unwrap());
    z.start_file("overrides/a.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    z.write_all(b"new").unwrap();
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

fn old() -> Vec<Tracked> {
    vec![Tracked {
        path: "a.txt".into(),
        sha1: hex::encode(sha1::Sha1::digest(b"old")),
        is_override: true,
    }]
}

fn dl() -> Downloader {
    Downloader::new("test", DownloaderConfig::default()).unwrap()
}

/// RR-003: a file edited after the update was planned (here, during the
/// last progress callback, after planning and before the files are moved)
/// is never overwritten; the update fails and changes nothing.
#[tokio::test]
async fn an_edit_after_planning_fails_the_update_and_is_kept() {
    let g = tempfile::tempdir().unwrap();
    let a = g.path().join("a.txt");
    std::fs::write(&a, b"old").unwrap();
    let edit = a.clone();
    let z = archive();
    let r = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &old(),
        &CancellationToken::new(),
        move |_| std::fs::write(&edit, b"user edit during download").unwrap(),
    )
    .await;
    assert!(r.is_err(), "the update must not go ahead over the edit");
    assert_eq!(std::fs::read(&a).unwrap(), b"user edit during download");
    assert!(!g.path().join(packupdate::STAGING).exists());
}

/// RR-004: an update that was applied but never recorded is undone by
/// recovery, even when the database already records the same pack version
/// (only the update's own id acknowledges it).
#[tokio::test]
async fn recovery_undoes_an_unrecorded_update_to_the_same_version() {
    for acknowledged in [None, Some("0123456789abcdef0123456789abcdef")] {
        let g = tempfile::tempdir().unwrap();
        std::fs::write(g.path().join("a.txt"), b"old").unwrap();
        let z = archive();
        let (_, pending) = packupdate::update(
            &dl(),
            g.path(),
            z.path(),
            &pack(),
            &old(),
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert_ne!(Some(pending.id()), acknowledged);
        drop(pending); // the process exits before the database is written
        let recovered = packupdate::recover(g.path(), acknowledged).unwrap();
        assert_eq!(recovered.rolled_back, 1, "{acknowledged:?}");
        assert_eq!(recovered.cleaned, 0, "{acknowledged:?}");
        assert_eq!(std::fs::read(g.path().join("a.txt")).unwrap(), b"old");
        assert!(!g.path().join(packupdate::STAGING).exists());
    }
}

/// RR-004: the update's own id, once recorded, keeps it.
#[tokio::test]
async fn recovery_keeps_an_update_recorded_by_its_id() {
    let g = tempfile::tempdir().unwrap();
    std::fs::write(g.path().join("a.txt"), b"old").unwrap();
    let z = archive();
    let (_, pending) = packupdate::update(
        &dl(),
        g.path(),
        z.path(),
        &pack(),
        &old(),
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    let id = pending.id().to_owned();
    drop(pending); // recorded, then the process exits before cleaning up
    let recovered = packupdate::recover(g.path(), Some(&id)).unwrap();
    assert_eq!(recovered.cleaned, 1);
    assert_eq!(recovered.rolled_back, 0);
    assert_eq!(std::fs::read(g.path().join("a.txt")).unwrap(), b"new");
    assert!(!g.path().join(packupdate::STAGING).exists());
}

/// RR-002: when a temp file's folder is swapped for a junction, a failed
/// persist doesn't clean up through it: the outside file survives.
#[cfg(windows)]
#[test]
fn a_failed_persist_after_a_parent_swap_keeps_the_outside_file() {
    use nc_launcher::confined::Confined;
    let g = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let c = Confined::new(g.path()).unwrap();
    let (file, tmp) = c.temp_for("cfg/x.txt").unwrap();
    drop(file);
    std::fs::rename(g.path().join("cfg"), g.path().join("real-cfg")).unwrap();
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(g.path().join("cfg"))
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sentinel = outside.path().join(tmp.file_name().unwrap());
    std::fs::write(&sentinel, b"external sentinel").unwrap();
    assert!(c.persist(&tmp, "cfg/x.txt").is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"external sentinel");
    assert!(!outside.path().join("x.txt").exists());
    std::fs::remove_dir(g.path().join("cfg")).unwrap();
}
