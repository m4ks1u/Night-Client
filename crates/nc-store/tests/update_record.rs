//! Recording a modpack update is one transaction (re-review RR-001, RR-004):
//! the pack, its files, the instance's versions and the update id commit
//! together or not at all.

#![allow(clippy::unwrap_used)]

use nc_store::{InstancePatch, Loader, ModpackRecord, NewInstance, PackFileRecord, Store};

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Store,
    String,
    ModpackRecord,
) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("store.db");
    let s = Store::open(&p).unwrap();
    let inst = s
        .create_instance(
            NewInstance {
                name: "Fixture".into(),
                game_version: "1.21.1".into(),
                loader: Loader::Fabric,
                loader_version: Some("0.16.0".into()),
                max_memory_mb: None,
            },
            |_| false,
        )
        .unwrap();
    let old = ModpackRecord {
        project_id: Some("project".into()),
        version_id: Some("v1".into()),
        name: "Fixture".into(),
        version_number: "1".into(),
        icon_url: None,
        installed_at: 0,
    };
    let files = [PackFileRecord {
        path: "mods/a.jar".into(),
        sha1: "a".repeat(40),
        is_override: false,
    }];
    s.set_modpack(&inst.id, &old, &files).unwrap();
    (d, p, s, inst.id, old)
}

fn v2(old: &ModpackRecord) -> ModpackRecord {
    ModpackRecord {
        version_id: Some("v2".into()),
        version_number: "2".into(),
        ..old.clone()
    }
}

fn patch() -> InstancePatch {
    InstancePatch {
        game_version: Some("1.21.2".into()),
        loader_version: Some("0.17.0".into()),
        ..Default::default()
    }
}

fn new_files() -> Vec<PackFileRecord> {
    vec![PackFileRecord {
        path: "mods/b.jar".into(),
        sha1: "b".repeat(40),
        is_override: false,
    }]
}

#[test]
fn an_update_records_everything_together() {
    let (_d, _p, s, id, old) = fixture();
    assert_eq!(s.modpack_update_id(&id).unwrap(), None);
    let inst = s
        .record_modpack_update(&id, &v2(&old), &new_files(), patch(), "abc123")
        .unwrap();
    assert_eq!(inst.game_version, "1.21.2");
    assert_eq!(inst.loader_version.as_deref(), Some("0.17.0"));
    assert_eq!(
        s.get_modpack(&id).unwrap().unwrap().version_id.as_deref(),
        Some("v2")
    );
    let files: Vec<String> = s
        .modpack_files(&id)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(files, ["mods/b.jar"]);
    assert_eq!(s.modpack_update_id(&id).unwrap().as_deref(), Some("abc123"));
}

/// the independent review's RR-001 reproduction with the fixed behavior: the instance write
/// fails, and nothing of the update is recorded.
#[test]
fn a_failing_instance_write_records_nothing() {
    let (_d, p, s, id, old) = fixture();
    let conn = rusqlite::Connection::open(&p).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_update BEFORE UPDATE ON instances BEGIN SELECT RAISE(FAIL, 'injected instance write failure'); END;",
    )
    .unwrap();
    let r = s.record_modpack_update(&id, &v2(&old), &new_files(), patch(), "abc123");
    assert!(r.is_err());
    let rec = s.get_modpack(&id).unwrap().unwrap();
    assert_eq!(
        rec.version_id.as_deref(),
        Some("v1"),
        "pack record unchanged"
    );
    let files: Vec<String> = s
        .modpack_files(&id)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(files, ["mods/a.jar"], "file list unchanged");
    assert_eq!(s.modpack_update_id(&id).unwrap(), None, "no acknowledgment");
    let inst = s.get_instance(&id).unwrap();
    assert_eq!(
        (inst.game_version.as_str(), inst.loader_version.as_deref()),
        ("1.21.1", Some("0.16.0"))
    );
}

/// A failure in the middle of the file list rolls back the pack row too.
#[test]
fn a_failing_file_row_records_nothing() {
    let (_d, _p, s, id, old) = fixture();
    let bad = vec![
        new_files()[0].clone(),
        PackFileRecord {
            path: "mods/c.jar".into(),
            sha1: "short".into(), // violates the CHECK constraint
            is_override: false,
        },
    ];
    assert!(
        s.record_modpack_update(&id, &v2(&old), &bad, patch(), "abc123")
            .is_err()
    );
    assert_eq!(
        s.get_modpack(&id).unwrap().unwrap().version_id.as_deref(),
        Some("v1")
    );
    assert_eq!(s.get_instance(&id).unwrap().game_version, "1.21.1");
    assert_eq!(s.modpack_update_id(&id).unwrap(), None);
}

#[test]
fn two_updates_to_the_same_version_have_different_acknowledgments() {
    let (_d, _p, s, id, old) = fixture();
    s.record_modpack_update(&id, &v2(&old), &new_files(), patch(), "first")
        .unwrap();
    s.record_modpack_update(
        &id,
        &v2(&old),
        &new_files(),
        InstancePatch::default(),
        "second",
    )
    .unwrap();
    assert_eq!(s.modpack_update_id(&id).unwrap().as_deref(), Some("second"));
}

#[test]
fn update_ids_are_validated() {
    let (_d, _p, s, id, old) = fixture();
    for bad in ["", "has space", "../x", &"x".repeat(65)] {
        assert!(
            s.record_modpack_update(&id, &v2(&old), &[], patch(), bad)
                .is_err(),
            "{bad}"
        );
    }
    assert_eq!(
        s.get_modpack(&id).unwrap().unwrap().version_id.as_deref(),
        Some("v1")
    );
}

/// `update_instance` is now atomic too: a later field failing keeps the earlier one.
#[test]
fn update_instance_applies_all_fields_or_none() {
    let (_d, _p, s, id, _) = fixture();
    let r = s.update_instance(
        &id,
        InstancePatch {
            name: Some("Renamed".into()),
            game_version: Some(String::new()), // invalid
            ..Default::default()
        },
    );
    assert!(r.is_err());
    assert_eq!(s.get_instance(&id).unwrap().name, "Fixture");
}

/// Importing again (set_modpack) clears an earlier acknowledgment.
#[test]
fn an_import_has_no_acknowledgment() {
    let (_d, _p, s, id, old) = fixture();
    s.record_modpack_update(&id, &v2(&old), &new_files(), patch(), "abc")
        .unwrap();
    s.set_modpack(&id, &old, &[]).unwrap();
    assert_eq!(s.modpack_update_id(&id).unwrap(), None);
}

/// A v8 database (Stage 14) with a modpack upgrades to v9 with its data kept
/// and no acknowledgment.
#[test]
fn v8_database_upgrades_to_v9_keeping_modpacks() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("nc.db");
    let id = {
        let s = Store::open_with_migrations(&p, &nc_store::migrations::MIGRATIONS[..8]).unwrap();
        assert_eq!(s.schema_version().unwrap(), 8);
        let inst = s
            .create_instance(
                NewInstance {
                    name: "Eight".into(),
                    game_version: "1.21.1".into(),
                    loader: Loader::Vanilla,
                    loader_version: None,
                    max_memory_mb: None,
                },
                |_| false,
            )
            .unwrap();
        let rec = ModpackRecord {
            project_id: Some("p".into()),
            version_id: Some("v1".into()),
            name: "Eight".into(),
            version_number: "1".into(),
            icon_url: None,
            installed_at: 3,
        };
        s.set_modpack(&inst.id, &rec, &new_files()).unwrap();
        inst.id
    };
    let s = Store::open(&p).unwrap();
    assert_eq!(s.schema_version().unwrap(), 9);
    assert_eq!(
        s.get_modpack(&id).unwrap().unwrap().version_id.as_deref(),
        Some("v1")
    );
    assert_eq!(s.modpack_files(&id).unwrap().len(), 1);
    assert_eq!(s.modpack_update_id(&id).unwrap(), None);
}
