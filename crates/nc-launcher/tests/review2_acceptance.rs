//! Independent cross-layer checks for update files, SQL metadata and recovery acknowledgment.
#![allow(clippy::unwrap_used)]
use nc_core::{loader::LoaderKind, mrpack::Pack};
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    modrinth::{modpack::Tracked, packupdate},
};
use nc_store::{InstancePatch, Loader, ModpackRecord, NewInstance, PackFileRecord, Store};
use sha1::Digest as _;
use std::io::Write;
struct Fixture {
    store: Store,
    dir: tempfile::TempDir,
    instance: String,
    archive: tempfile::NamedTempFile,
    old: Vec<Tracked>,
    record: ModpackRecord,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        std::fs::create_dir(&game).unwrap();
        std::fs::write(game.join("a.txt"), b"old").unwrap();
        let store = Store::open(&dir.path().join("store.db")).unwrap();
        let instance = store
            .create_instance(
                NewInstance {
                    name: "Independent fixture".into(),
                    game_version: "1.21.1".into(),
                    loader: Loader::Vanilla,
                    loader_version: None,
                    max_memory_mb: None,
                },
                |_| false,
            )
            .unwrap()
            .id;
        let old = vec![Tracked {
            path: "a.txt".into(),
            sha1: hex::encode(sha1::Sha1::digest(b"old")),
            is_override: true,
        }];
        let record = ModpackRecord {
            project_id: Some("fixture".into()),
            version_id: Some("v1".into()),
            name: "Fixture".into(),
            version_number: "1".into(),
            icon_url: None,
            installed_at: 0,
        };
        store
            .record_modpack_update(
                &instance,
                &record,
                &rows(&old),
                InstancePatch::default(),
                &"1".repeat(32),
            )
            .unwrap();
        let archive = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(archive.reopen().unwrap());
        zip.start_file("overrides/a.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"new").unwrap();
        zip.finish().unwrap();
        Self {
            dir,
            store,
            instance,
            archive,
            old,
            record,
        }
    }
    fn game(&self) -> std::path::PathBuf {
        self.dir.path().join("game")
    }
    async fn apply(&self) -> (packupdate::UpdateReport, packupdate::Pending) {
        let pack = Pack {
            name: "Fixture".into(),
            version_id: "2".into(),
            summary: None,
            minecraft: "1.21.2".into(),
            loader: LoaderKind::Vanilla,
            loader_version: None,
            files: vec![],
            skipped: 0,
        };
        let dl = Downloader::new("independent-review", DownloaderConfig::default()).unwrap();
        packupdate::update(
            &dl,
            &self.game(),
            self.archive.path(),
            &pack,
            &self.old,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap()
    }
    fn next_record(&self) -> ModpackRecord {
        ModpackRecord {
            version_id: Some("v2".into()),
            version_number: "2".into(),
            ..self.record.clone()
        }
    }
}
fn rows(files: &[Tracked]) -> Vec<PackFileRecord> {
    files
        .iter()
        .map(|f| PackFileRecord {
            path: f.path.clone(),
            sha1: f.sha1.clone(),
            is_override: f.is_override,
        })
        .collect()
}
#[tokio::test]
async fn independent_sql_commit_acknowledges_exact_filesystem_transaction() {
    let f = Fixture::new();
    let (report, pending) = f.apply().await;
    let id = pending.id().to_owned();
    f.store
        .record_modpack_update(
            &f.instance,
            &f.next_record(),
            &rows(&report.tracked),
            InstancePatch {
                game_version: Some("1.21.2".into()),
                ..Default::default()
            },
            &id,
        )
        .unwrap();
    drop(pending); // simulated exit after real SQL commit, before finish
    let reopened = Store::open(&f.dir.path().join("store.db")).unwrap();
    let ack = reopened.modpack_update_id(&f.instance).unwrap();
    assert_eq!(ack.as_deref(), Some(id.as_str()));
    assert_eq!(
        packupdate::recover(&f.game(), ack.as_deref())
            .unwrap()
            .cleaned,
        1
    );
    assert_eq!(std::fs::read(f.game().join("a.txt")).unwrap(), b"new");
    assert_eq!(
        reopened.get_instance(&f.instance).unwrap().game_version,
        "1.21.2"
    );
    assert_eq!(
        reopened
            .get_modpack(&f.instance)
            .unwrap()
            .unwrap()
            .version_id
            .as_deref(),
        Some("v2")
    );
    assert_eq!(
        reopened.modpack_files(&f.instance).unwrap()[0].sha1,
        hex::encode(sha1::Sha1::digest(b"new"))
    );
    assert!(!f.game().join(packupdate::STAGING).exists());
    assert_eq!(
        packupdate::recover(&f.game(), ack.as_deref()).unwrap(),
        packupdate::Recovered::default()
    );
}
#[tokio::test]
async fn independent_failed_sql_transaction_keeps_prior_ack_and_recovery_restores_files() {
    let f = Fixture::new();
    let (report, pending) = f.apply().await;
    let outcome = f.store.record_modpack_update(
        &f.instance,
        &f.next_record(),
        &rows(&report.tracked),
        InstancePatch {
            game_version: Some(String::new()),
            ..Default::default()
        },
        pending.id(),
    );
    assert!(
        outcome.is_err(),
        "validation inside the SQL transaction fails"
    );
    drop(pending); // simulated exit before the command's rollback worker
    let reopened = Store::open(&f.dir.path().join("store.db")).unwrap();
    let ack = reopened.modpack_update_id(&f.instance).unwrap();
    assert_eq!(ack, Some("1".repeat(32)));
    assert_eq!(
        packupdate::recover(&f.game(), ack.as_deref())
            .unwrap()
            .rolled_back,
        1
    );
    assert_eq!(std::fs::read(f.game().join("a.txt")).unwrap(), b"old");
    assert_eq!(
        reopened.get_instance(&f.instance).unwrap().game_version,
        "1.21.1"
    );
    assert_eq!(
        reopened
            .get_modpack(&f.instance)
            .unwrap()
            .unwrap()
            .version_id
            .as_deref(),
        Some("v1")
    );
    assert_eq!(
        reopened.modpack_files(&f.instance).unwrap()[0].sha1,
        f.old[0].sha1
    );
    assert!(!f.game().join(packupdate::STAGING).exists());
}
