//! Modpack (`.mrpack`) support: search facets and verified file downloads
//! against a mock server, plus an ignored test that installs a real pack
//! from Modrinth into a temporary folder.

#![allow(clippy::unwrap_used)]

use nc_core::mrpack::PackFile;
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    modrinth::{
        Modrinth,
        api::{SortIndex, pick_modpack_version},
        modpack,
    },
};
use serde_json::json;
use sha1::Digest as _;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

const UA: &str = "NightClient/test (+https://example.invalid)";

#[tokio::test]
async fn modpack_search_uses_modpack_type_and_supported_loaders() {
    let s = MockServer::start().await;
    let facets = r#"[["project_type:modpack"],["categories:fabric","categories:quilt","categories:neoforge","categories:forge"],["client_side:required","client_side:optional"]]"#;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("facets", facets))
        .and(query_param("index", "downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits":[{"project_id":"1KVo5zza","slug":"fabulously-optimized","title":"FO","description":"d","author":"a"}],
            "offset":0,"limit":20,"total_hits":1
        })))
        .expect(1)
        .mount(&s)
        .await;
    let m = Modrinth::with_base(UA, &s.uri()).unwrap();
    let r = m
        .search_modpacks("fo", SortIndex::Downloads, 0, 20, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(r.hits[0].project_id, "1KVo5zza");
}

#[test]
fn picks_newest_release_with_an_mrpack_for_a_supported_loader() {
    let v = |id: &str, ty: &str, loader: &str, file: &str| {
        serde_json::from_value::<nc_launcher::modrinth::api::Version>(json!({
            "id": id, "project_id": "p", "name": id, "version_number": id, "version_type": ty,
            "loaders": [loader], "game_versions": ["1.21.1"],
            "files": [{"hashes": {"sha1": "a", "sha512": "b"}, "url": format!("https://cdn.modrinth.com/data/p/{file}"), "filename": file, "primary": true, "size": 1}]
        }))
        .unwrap()
    };
    let list = vec![
        v("beta", "beta", "fabric", "p.mrpack"),
        v("other", "release", "liteloader", "p.mrpack"),
        v("zip", "release", "fabric", "p.zip"),
        v("ok", "release", "fabric", "p.mrpack"),
        v("old", "release", "fabric", "p.mrpack"),
    ];
    assert_eq!(pick_modpack_version(&list).unwrap().id, "ok");
    assert_eq!(pick_modpack_version(&list[..1]).unwrap().id, "beta");
    assert!(pick_modpack_version(&list[1..3]).is_none());
}

fn file(path: &str, url: String, data: &[u8]) -> PackFile {
    PackFile {
        path: path.into(),
        url,
        sha1: hex::encode(sha1::Sha1::digest(data)),
        sha512: hex::encode(sha2::Sha512::digest(data)),
        size: data.len() as u64,
        optional: false,
    }
}

#[tokio::test]
async fn pack_files_are_verified_by_sha512_too() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/a.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"mod-a".to_vec()))
        .mount(&s)
        .await;
    let dl = Downloader::new(UA, DownloaderConfig::default())
        .unwrap()
        .allow_insecure_for_tests();
    let d = tempfile::tempdir().unwrap();
    let c = CancellationToken::new();
    let good = file("mods/a.jar", format!("{}/a.jar", s.uri()), b"mod-a");
    modpack::download_files(&dl, d.path(), std::slice::from_ref(&good), &c, |_| {})
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(d.path().join("mods/a.jar")).unwrap(),
        b"mod-a"
    );

    // Right size and SHA-1 but a different SHA-512: refused and deleted.
    let d2 = tempfile::tempdir().unwrap();
    let mut bad = good.clone();
    bad.sha512 = "0".repeat(128);
    let e = modpack::download_files(&dl, d2.path(), &[bad], &c, |_| {})
        .await
        .unwrap_err();
    assert!(e.to_string().contains("SHA-512"), "{e}");
    assert!(!d2.path().join("mods/a.jar").exists());

    // Paths are checked again here, not only when the index is parsed.
    let mut evil = good;
    evil.path = "../a.jar".into();
    assert!(
        modpack::download_files(&dl, d2.path(), &[evil], &c, |_| {})
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to the real Modrinth API and CDN"]
async fn real_modpack_install() {
    let m = Modrinth::new(&nc_launcher::user_agent()).unwrap();
    let dl = Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default()).unwrap();
    let c = CancellationToken::new();
    let hits = m
        .search_modpacks("", SortIndex::Downloads, 0, 20, &c)
        .await
        .unwrap();
    assert!(!hits.hits.is_empty());
    let cache = tempfile::tempdir().unwrap();
    // The smallest of the first packs that Night Client can install.
    let mut best = None;
    for h in hits.hits.iter().take(8) {
        let versions = m.all_versions(&h.project_id, &c).await.unwrap();
        let Some(v) = pick_modpack_version(&versions) else {
            continue;
        };
        let f = v.primary_file().unwrap();
        let p = modpack::fetch_pack(&dl, cache.path(), f, &c).await.unwrap();
        let Ok(pack) = modpack::read_pack(&p) else {
            continue;
        };
        println!(
            "{}: {} files, {} bytes, {:?} {}",
            h.title,
            pack.files.len(),
            pack.download_bytes(),
            pack.loader,
            pack.minecraft
        );
        if best
            .as_ref()
            .is_none_or(|(_, b): &(_, nc_core::mrpack::Pack)| {
                pack.download_bytes() < b.download_bytes()
            })
        {
            best = Some((p, pack));
        }
    }
    let (archive, pack) = best.expect("an installable pack");
    let game = tempfile::tempdir().unwrap();
    let s = modpack::download_files(&dl, game.path(), &pack.files, &c, |_| {})
        .await
        .unwrap();
    assert_eq!(s.downloaded_files as usize, pack.files.len());
    let n =
        modpack::extract_overrides(&archive, game.path(), modpack::MAX_OVERRIDES_BYTES).unwrap();
    println!(
        "installed {}: {} files, {} overrides",
        pack.name,
        pack.files.len(),
        n.len()
    );
    for f in &pack.files {
        assert!(game.path().join(&f.path).is_file(), "{}", f.path);
    }
}
