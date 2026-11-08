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
    assert_eq!(
        tables,
        [
            "accounts",
            "download_history",
            "instance_mods",
            "instances",
            "settings",
            "sqlite_sequence"
        ]
    );
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
        reduce_motion: true,
        gpu_acceleration: true,
        check_updates: false,
        onboarded: true,
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

// ----------------------------------------------------- real upgrade path ----

#[test]
fn shipped_v1_database_upgrades_to_latest_keeping_instances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nc.db");
    // Seed with raw SQL as v1-era code would have written it (today's API
    // reads columns that a v1 schema does not have yet).
    let id = "11111111-2222-4333-8444-555555555555";
    {
        drop(Store::open_with_migrations(&path, &migrations::MIGRATIONS[..1]).unwrap());
        let c = Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO instances (id, name, dir_name, game_version, created_at) VALUES (?1, 'From v1', 'From v1', '1.21.1', 1)",
            [id],
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(
        s.schema_version().unwrap(),
        migrations::MIGRATIONS.len() as u32
    );
    assert_eq!(s.get_instance(id).unwrap().name, "From v1");
    assert!(dir.path().join("nc.db.v1.bak").is_file());
    assert!(s.list_downloads(10).unwrap().is_empty());
}

// ------------------------------------------------------------- downloads ----

use nc_store::{DownloadRecord, DownloadStatus, HISTORY_LIMIT};

fn rec(target: &str, status: DownloadStatus) -> DownloadRecord {
    DownloadRecord {
        id: 0,
        kind: "version".into(),
        target: target.into(),
        status,
        repair: false,
        queued_at: 1,
        started_at: 2,
        finished_at: 3,
        files_downloaded: 10,
        files_verified: 5,
        bytes_downloaded: 1024,
        repaired: vec![],
        error: None,
    }
}

#[test]
fn download_history_roundtrip_order_and_clear() {
    let s = Store::open_in_memory().unwrap();
    s.record_download(&rec("1.21.1", DownloadStatus::Completed))
        .unwrap();
    let mut failed = rec("26.3", DownloadStatus::Failed);
    failed.error = Some("network".into());
    failed.repair = true;
    failed.repaired = (0..500).map(|i| format!("libraries/x/{i}.jar")).collect();
    s.record_download(&failed).unwrap();

    let list = s.list_downloads(50).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].target, "26.3", "newest first");
    assert_eq!(list[0].status, DownloadStatus::Failed);
    assert!(list[0].repair);
    assert_eq!(list[0].repaired.len(), 200, "repaired list is capped");
    assert_eq!(list[0].error.as_deref(), Some("network"));
    assert_eq!(s.clear_downloads().unwrap(), 2);
    assert!(s.list_downloads(50).unwrap().is_empty());
}

#[test]
fn download_history_is_pruned() {
    let s = Store::open_in_memory().unwrap();
    for i in 0..(HISTORY_LIMIT + 15) {
        s.record_download(&rec(&format!("v{i}"), DownloadStatus::Completed))
            .unwrap();
    }
    let list = s.list_downloads(1000).unwrap();
    assert_eq!(list.len() as i64, HISTORY_LIMIT);
    assert_eq!(list[0].target, format!("v{}", HISTORY_LIMIT + 14));
}

#[test]
fn invalid_download_record_is_rejected() {
    let s = Store::open_in_memory().unwrap();
    let mut r = rec("x", DownloadStatus::Completed);
    r.kind = String::new();
    assert!(s.record_download(&r).is_err());
}

#[test]
fn java_override_set_and_cleared() {
    let s = Store::open_in_memory().unwrap();
    let a = s.create_instance(new("J"), |_| false).unwrap();
    let p = InstancePatch {
        java_path: Some(Some(r"C:\Java\jdk-21\bin\javaw.exe".into())),
        ..Default::default()
    };
    assert_eq!(
        s.update_instance(&a.id, p).unwrap().java_path.as_deref(),
        Some(r"C:\Java\jdk-21\bin\javaw.exe")
    );
    let p = InstancePatch {
        java_path: Some(None),
        ..Default::default()
    };
    assert_eq!(s.update_instance(&a.id, p).unwrap().java_path, None);
    let p = InstancePatch {
        java_path: Some(Some(String::new())),
        ..Default::default()
    };
    assert!(s.update_instance(&a.id, p).is_err());
}

// -------------------------------------------------------------- accounts ----

const A: &str = "0123456789abcdef0123456789abcdef";
const B: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn accounts_single_active_switch_and_remove() {
    let s = Store::open_in_memory().unwrap();
    assert!(s.active_account().unwrap().is_none());
    s.upsert_active_account(A, "Alex").unwrap();
    s.upsert_active_account(&B.to_uppercase(), "Steve").unwrap();
    let list = s.list_accounts().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].uuid.as_str(), list[0].active),
        (B, true),
        "uuid normalised, newest active first"
    );
    assert!(!list[1].active);

    assert_eq!(s.set_active_account(A).unwrap().name, "Alex");
    assert_eq!(s.active_account().unwrap().unwrap().uuid, A);
    assert!(matches!(
        s.set_active_account("00000000000000000000000000000000"),
        Err(StoreError::NotFound)
    ));

    // Re-signing in updates the name instead of duplicating.
    s.upsert_active_account(A, "AlexRenamed").unwrap();
    assert_eq!(s.list_accounts().unwrap().len(), 2);

    // Removing the active account promotes the remaining one.
    let next = s.remove_account(A).unwrap().unwrap();
    assert_eq!(next.uuid, B);
    assert!(next.active);
    assert_eq!(s.remove_account(B).unwrap(), None);
    assert!(matches!(s.remove_account(B), Err(StoreError::NotFound)));
}

#[test]
fn account_input_is_validated() {
    let s = Store::open_in_memory().unwrap();
    assert!(s.upsert_active_account("../../x", "Alex").is_err());
    assert!(s.upsert_active_account(A, "").is_err());
    assert!(
        s.upsert_active_account(A, "Robert'); DROP TABLE accounts;--")
            .is_err()
    );
}

#[test]
fn jvm_args_are_stored_and_bounded() {
    let s = Store::open_in_memory().unwrap();
    let i = s.create_instance(new("J"), |_| false).unwrap();
    assert!(i.jvm_args.is_empty());
    let args = vec![
        "-XX:+UseG1GC".to_string(),
        "-Dfile.encoding=UTF-8".to_string(),
    ];
    let p = InstancePatch {
        jvm_args: Some(args.clone()),
        ..Default::default()
    };
    assert_eq!(s.update_instance(&i.id, p).unwrap().jvm_args, args);
    let p = InstancePatch {
        jvm_args: Some(vec!["-Dx=1".into(); 65]),
        ..Default::default()
    };
    assert!(s.update_instance(&i.id, p).is_err());
}

#[test]
fn loader_version_and_game_dir_patches() {
    let s = Store::open_in_memory().unwrap();
    let vanilla = s.create_instance(new("V"), |_| false).unwrap();
    assert_eq!(vanilla.game_dir, None);
    let p = InstancePatch {
        loader_version: Some("0.17.2".into()),
        ..Default::default()
    };
    assert!(
        s.update_instance(&vanilla.id, p).is_err(),
        "vanilla has no loader version"
    );

    let mut nf = new("N");
    nf.loader = Loader::Neoforge;
    nf.loader_version = Some("21.1.200".into());
    let nf = s.create_instance(nf, |_| false).unwrap();
    let p = InstancePatch {
        loader_version: Some("21.1.265".into()),
        ..Default::default()
    };
    assert_eq!(
        s.update_instance(&nf.id, p)
            .unwrap()
            .loader_version
            .as_deref(),
        Some("21.1.265")
    );

    let p = InstancePatch {
        game_dir: Some(Some(r"D:\Games\N".into())),
        ..Default::default()
    };
    assert_eq!(
        s.update_instance(&nf.id, p).unwrap().game_dir.as_deref(),
        Some(r"D:\Games\N")
    );
    let p = InstancePatch {
        game_dir: Some(None),
        ..Default::default()
    };
    assert_eq!(s.update_instance(&nf.id, p).unwrap().game_dir, None);
}

// ------------------------------------------------------------------ mods ----

use nc_store::{ModRecord, ModSource};

fn modrec(instance: &str, file: &str) -> ModRecord {
    ModRecord {
        instance_id: instance.into(),
        file_name: file.into(),
        source: ModSource::Modrinth,
        project_id: Some("AANobbMI".into()),
        version_id: Some("v1".into()),
        title: "Sodium".into(),
        version_number: Some("0.8.13".into()),
        icon_url: None,
        sha1: "a".repeat(40),
        sha512: "b".repeat(128),
        size: 10,
        enabled: true,
        required_by: None,
        installed_at: 0,
    }
}

#[test]
fn mods_crud_and_cascade_on_instance_delete() {
    let s = Store::open_in_memory().unwrap();
    let i = s.create_instance(new("M"), |_| false).unwrap();
    s.upsert_mod(&modrec(&i.id, "sodium.jar")).unwrap();
    let mut other = modrec(&i.id, "lithium.jar");
    other.title = "Lithium".into();
    other.source = ModSource::Local;
    s.upsert_mod(&other).unwrap();
    let list = s.list_mods(&i.id).unwrap();
    assert_eq!(
        list.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
        ["Lithium", "Sodium"]
    );
    assert!(list[1].installed_at > 0);

    s.set_mod_enabled(&i.id, "sodium.jar", false).unwrap();
    assert!(!s.list_mods(&i.id).unwrap()[1].enabled);
    assert!(s.set_mod_enabled(&i.id, "nope.jar", true).is_err());
    let mut bad = modrec(&i.id, "x.jar");
    bad.sha512 = "short".into();
    assert!(s.upsert_mod(&bad).is_err());

    s.remove_mod(&i.id, "lithium.jar").unwrap();
    assert_eq!(s.list_mods(&i.id).unwrap().len(), 1);
    s.delete_instance(&i.id).unwrap();
    assert!(
        s.list_mods(&i.id).unwrap().is_empty(),
        "rows cascade with the instance"
    );
}
