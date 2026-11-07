#![allow(clippy::unwrap_used)]

use nc_store::{
    AppSettings, InstancePatch, Loader, NewInstance, Store, StoreError,
    migrations::{self, Migration},
};
use rusqlite::Connection;

fn new(name: &str) -> NewInstance {
    NewInstance {
        name: name.into(),
        game_version: "1.21.1".into(),
        loader: Loader::Vanilla,
        loader_version: None,
        max_memory_mb: None,
    }
}

// ------------------------------------------------------------ migrations ----

#[test]
fn fresh_database_reaches_latest_version() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(
        s.schema_version().unwrap(),
        migrations::MIGRATIONS.len() as u32
    );
}

#[test]
fn reopening_is_idempotent_and_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nc.db");
    let id = {
        let s = Store::open(&path).unwrap();
        s.create_instance(new("Keep me"), |_| false).unwrap().id
    };
    let s = Store::open(&path).unwrap();
    assert_eq!(s.get_instance(&id).unwrap().name, "Keep me");
    // No backup is written when nothing needs migrating.
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".bak")
        })
        .count();
    assert_eq!(backups, 0);
}

const M1: Migration = Migration {
    version: 1,
    name: "one",
    sql: "CREATE TABLE t (a INTEGER) STRICT;",
};
const M2: Migration = Migration {
    version: 2,
    name: "two",
    sql: "ALTER TABLE t ADD COLUMN b TEXT; INSERT INTO t (a, b) VALUES (1, 'x');",
};
const M2_BROKEN: Migration = Migration {
    version: 2,
    name: "broken",
    sql: "ALTER TABLE t ADD COLUMN b TEXT; THIS IS NOT SQL;",
};

#[test]
fn migrations_apply_stepwise_preserving_data() {
    let mut c = Connection::open_in_memory().unwrap();
    assert_eq!(migrations::migrate(&mut c, &[M1]).unwrap(), 1);
    c.execute("INSERT INTO t (a) VALUES (7)", []).unwrap();
    assert_eq!(migrations::migrate(&mut c, &[M1, M2]).unwrap(), 2);
    let n: i64 = c
        .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2, "row from v1 survives and v2 data is added");
}

#[test]
fn failed_migration_rolls_back_to_last_good_version() {
    let mut c = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut c, &[M1]).unwrap();
    let err = migrations::migrate(&mut c, &[M1, M2_BROKEN]).unwrap_err();
    assert!(
        matches!(err, StoreError::Migration { version: 2, .. }),
        "{err}"
    );
    assert_eq!(migrations::user_version(&c).unwrap(), 1);
    // The partial ALTER TABLE was rolled back too.
    let cols: i64 = c
        .query_row("SELECT count(*) FROM pragma_table_info('t')", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(cols, 1);
}

#[test]
fn existing_database_is_backed_up_before_migrating() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nc.db");
    drop(Store::open_with_migrations(&path, &[M1]).unwrap());
    let s = Store::open_with_migrations(&path, &[M1, M2]).unwrap();
    assert_eq!(s.schema_version().unwrap(), 2);
    let backup = dir.path().join("nc.db.v1.bak");
    assert!(backup.is_file(), "backup written");
    let c = Connection::open(&backup).unwrap();
    assert_eq!(
        migrations::user_version(&c).unwrap(),
        1,
        "backup holds the pre-migration schema"
    );
}

#[test]
fn newer_schema_is_refused() {
    let mut c = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut c, &[M1, M2]).unwrap();
    let err = migrations::migrate(&mut c, &[M1]).unwrap_err();
    assert!(matches!(
        err,
        StoreError::NewerSchema {
            found: 2,
            supported: 1
        }
    ));
}

#[test]
fn newer_schema_file_is_not_modified() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nc.db");
    {
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
    }
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::NewerSchema { found: 99, .. })
    ));
    let c = Connection::open(&path).unwrap();
    assert_eq!(migrations::user_version(&c).unwrap(), 99);
}

#[test]
fn shipped_migration_sql_is_valid_on_an_empty_db() {
    let mut c = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut c, migrations::MIGRATIONS).unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tables, ["instances", "settings"]);
}

// ------------------------------------------------------------- instances ----

#[test]
fn create_list_rename_delete() {
    let s = Store::open_in_memory().unwrap();
    let a = s.create_instance(new("  Alpha  "), |_| false).unwrap();
    assert_eq!(a.name, "Alpha");
    assert_eq!(a.dir_name, "Alpha");
    let b = s.create_instance(new("Beta"), |_| false).unwrap();
    assert_eq!(s.list_instances().unwrap().len(), 2);

    let renamed = s
        .update_instance(
            &a.id,
            InstancePatch {
                name: Some("Gamma".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.name, "Gamma");
    assert_eq!(renamed.dir_name, "Alpha", "renaming never moves the folder");

    s.delete_instance(&b.id).unwrap();
    assert!(matches!(s.get_instance(&b.id), Err(StoreError::NotFound)));
    assert!(matches!(
        s.delete_instance(&b.id),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn folder_names_are_unique_case_insensitively_and_respect_disk() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(
        s.create_instance(new("World"), |_| false).unwrap().dir_name,
        "World"
    );
    assert_eq!(
        s.create_instance(new("world"), |_| false).unwrap().dir_name,
        "world-2"
    );
    assert_eq!(
        s.create_instance(new("Disk"), |d| d == "Disk")
            .unwrap()
            .dir_name,
        "Disk-2"
    );
    assert_eq!(
        s.create_instance(new("../../Windows"), |_| false)
            .unwrap()
            .dir_name,
        "Windows"
    );
}

#[test]
fn invalid_input_is_rejected() {
    let s = Store::open_in_memory().unwrap();
    assert!(matches!(
        s.create_instance(new("   "), |_| false),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.create_instance(new(&"x".repeat(65)), |_| false),
        Err(StoreError::Invalid(_))
    ));
    let mut bad = new("ok");
    bad.game_version = "../1.21".into();
    assert!(s.create_instance(bad, |_| false).is_err());
    let mut mem = new("ok");
    mem.max_memory_mb = Some(100);
    assert!(s.create_instance(mem, |_| false).is_err());
    let a = s.create_instance(new("ok"), |_| false).unwrap();
    assert!(
        s.update_instance(
            &a.id,
            InstancePatch {
                max_memory_mb: Some(Some(70000)),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(matches!(
        s.update_instance(
            "nope",
            InstancePatch {
                name: Some("x".into()),
                ..Default::default()
            }
        ),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn recently_played_sorts_first_and_play_time_accumulates() {
    let s = Store::open_in_memory().unwrap();
    let a = s.create_instance(new("A"), |_| false).unwrap();
    let b = s.create_instance(new("B"), |_| false).unwrap();
    s.record_play(&a.id, 1_000, 60).unwrap();
    s.record_play(&a.id, 2_000, 30).unwrap();
    let list = s.list_instances().unwrap();
    assert_eq!(list[0].id, a.id);
    assert_eq!(list[0].play_time_seconds, 90);
    assert_eq!(list[0].last_played_at, Some(2_000));
    assert_eq!(list[1].id, b.id);
}

// -------------------------------------------------------------- settings ----

#[test]
fn settings_roundtrip_defaults_and_corruption() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.app_settings(), AppSettings::default());
    let custom = AppSettings {
        default_max_memory_mb: 6144,
        download_concurrency: 4,
        hide_on_launch: true,
    };
    s.save_app_settings(&custom).unwrap();
    assert_eq!(s.app_settings(), custom);

    assert!(
        s.save_app_settings(&AppSettings {
            download_concurrency: 0,
            ..custom.clone()
        })
        .is_err()
    );

    // Partial JSON from an older version fills missing fields with defaults.
    s.set_setting("app", &serde_json::json!({ "defaultMaxMemoryMb": 2048 }))
        .unwrap();
    assert_eq!(s.app_settings().default_max_memory_mb, 2048);
    assert_eq!(s.app_settings().download_concurrency, 8);

    // Corrupt values fall back to defaults instead of breaking startup.
    s.set_setting("app", &"not an object").unwrap();
    assert_eq!(s.app_settings(), AppSettings::default());
}
