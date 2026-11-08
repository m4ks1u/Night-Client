//! Loader version listing against mock Fabric Meta / Quilt Meta / NeoForge Maven.

#![allow(clippy::unwrap_used)]

use nc_core::loader::LoaderKind;
use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    layout::Layout,
    loaders,
    meta::Meta,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn meta(server: &MockServer, dir: &std::path::Path) -> Meta {
    let cfg = DownloaderConfig {
        max_retries: 0,
        ..Default::default()
    };
    let mut m = Meta::new(
        Downloader::new("t", cfg)
            .unwrap()
            .allow_insecure_for_tests(),
        Layout::new(dir),
    );
    m.fabric_meta_url = server.uri();
    m.quilt_meta_url = server.uri();
    m.neoforge_maven_url = server.uri();
    m.forge_maven_url = server.uri();
    m
}

#[tokio::test]
async fn fabric_versions_and_unsupported_game_versions() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/versions/loader/1.21.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"loader":{"version":"0.17.3-beta.1","stable":false},"intermediary":{}},
            {"loader":{"version":"0.17.2","stable":true},"intermediary":{}},
            {"loader":{"version":"0.17.1","stable":false},"intermediary":{}}
        ])))
        .mount(&s)
        .await;
    Mock::given(path("/v2/versions/loader/1.5.2"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let m = meta(&s, dir.path());
    let c = CancellationToken::new();
    let v = loaders::versions(&m, LoaderKind::Fabric, "1.21.1", &c)
        .await
        .unwrap();
    // Meta marks only the newest release stable; older releases are still releases.
    let flags: Vec<_> = v.iter().map(|x| (x.version.as_str(), x.stable)).collect();
    assert_eq!(
        flags,
        [("0.17.3-beta.1", false), ("0.17.2", true), ("0.17.1", true)]
    );
    assert!(
        loaders::versions(&m, LoaderKind::Fabric, "1.5.2", &c)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        loaders::versions(&m, LoaderKind::Fabric, "../x", &c)
            .await
            .is_err(),
        "path injection refused"
    );
}

#[tokio::test]
async fn neoforge_versions_from_maven_metadata_with_offline_fallback() {
    let s = MockServer::start().await;
    let xml = "<metadata><versioning><versions>\
        <version>21.1.9</version><version>21.1.265</version><version>21.1.266-beta</version>\
        <version>21.4.1</version><version>26.3.0.101</version></versions></versioning></metadata>";
    Mock::given(path("/net/neoforged/neoforge/maven-metadata.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(xml))
        .up_to_n_times(1)
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let m = meta(&s, dir.path());
    let c = CancellationToken::new();
    let v = loaders::versions(&m, LoaderKind::NeoForge, "1.21.1", &c)
        .await
        .unwrap();
    let names: Vec<_> = v.iter().map(|x| (x.version.as_str(), x.stable)).collect();
    assert_eq!(
        names,
        [
            ("21.1.266-beta", false),
            ("21.1.265", true),
            ("21.1.9", true)
        ]
    );

    // Second call: the server now fails (mock exhausted → 404), cache is used.
    let again = loaders::versions(&m, LoaderKind::NeoForge, "26.3", &c)
        .await
        .unwrap();
    assert_eq!(again[0].version, "26.3.0.101");
    assert!(
        loaders::versions(&m, LoaderKind::Vanilla, "1.21.1", &c)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn quilt_versions_are_sorted_newest_first() {
    let s = MockServer::start().await;
    // Quilt Meta v3 returns loaders in no particular order.
    Mock::given(method("GET"))
        .and(path("/v3/versions/loader/1.21.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"loader":{"version":"0.20.0-beta.9","build":9},"hashed":{}},
            {"loader":{"version":"0.29.2","build":2},"hashed":{}},
            {"loader":{"version":"0.24.0","build":0},"hashed":{}},
            {"loader":{"version":"0.29.3-beta.1","build":1},"hashed":{}}
        ])))
        .mount(&s)
        .await;
    Mock::given(path("/v3/versions/loader/1.5.2"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let m = meta(&s, dir.path());
    let c = CancellationToken::new();
    let v = loaders::versions(&m, LoaderKind::Quilt, "1.21.1", &c)
        .await
        .unwrap();
    let flags: Vec<_> = v.iter().map(|x| (x.version.as_str(), x.stable)).collect();
    assert_eq!(
        flags,
        [
            ("0.29.3-beta.1", false),
            ("0.29.2", true),
            ("0.24.0", true),
            ("0.20.0-beta.9", false)
        ]
    );
    assert!(
        loaders::versions(&m, LoaderKind::Quilt, "1.5.2", &c)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn forge_versions_strip_the_game_version_and_skip_old_installers() {
    let s = MockServer::start().await;
    let xml = "<metadata><versioning><versions>\
        <version>1.20.1-47.4.5</version><version>1.20.1-47.4.29</version>\
        <version>1.16.5-36.2.42</version><version>1.21.1-52.1.21</version></versions></versioning></metadata>";
    Mock::given(path("/net/minecraftforge/forge/maven-metadata.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(xml))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let m = meta(&s, dir.path());
    let c = CancellationToken::new();
    let v = loaders::versions(&m, LoaderKind::Forge, "1.20.1", &c)
        .await
        .unwrap();
    let names: Vec<_> = v.iter().map(|x| x.version.as_str()).collect();
    assert_eq!(names, ["47.4.29", "47.4.5"]);
    assert!(
        loaders::versions(&m, LoaderKind::Forge, "1.16.5", &c)
            .await
            .unwrap()
            .is_empty(),
        "spec 0 installers are not offered"
    );
}
