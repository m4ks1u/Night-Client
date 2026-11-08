//! Modrinth client behaviour against a mock server, plus an ignored
//! real-API test that installs a mod with a required dependency.

#![allow(clippy::unwrap_used)]

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use nc_launcher::{
    CancellationToken,
    download::{Downloader, DownloaderConfig},
    modrinth::{
        Modrinth, ModrinthError,
        api::{SearchQuery, SortIndex},
        mods,
        plan::{self, pick_best},
    },
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{body_partial_json, header, method, path, query_param},
};

const UA: &str = "NightClient/test (+https://example.invalid)";

fn search_body() -> serde_json::Value {
    json!({"hits":[{"project_id":"AANobbMI","slug":"sodium","title":"Sodium","description":"fast","downloads":10,"follows":1,"author":"x","display_categories":["optimization"]}],"offset":0,"limit":20,"total_hits":1})
}

#[tokio::test]
async fn search_sends_user_agent_facets_and_caches() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(header("user-agent", UA))
        .and(query_param("index", "downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .expect(1)
        .mount(&s)
        .await;
    let m = Modrinth::with_base(UA, &s.uri()).unwrap();
    let c = CancellationToken::new();
    let r = m
        .search(
            &SearchQuery {
                query: "sod",
                loader: "fabric",
                minecraft: "1.21.1",
                category: Some("optimization"),
                sort: SortIndex::Downloads,
                offset: 0,
                limit: 20,
            },
            &c,
        )
        .await
        .unwrap();
    assert_eq!(r.hits[0].title, "Sodium");
    // Second identical search is served from cache (expect(1) verifies on drop).
    m.search(
        &SearchQuery {
            query: "sod",
            loader: "fabric",
            minecraft: "1.21.1",
            category: Some("optimization"),
            sort: SortIndex::Downloads,
            offset: 0,
            limit: 20,
        },
        &c,
    )
    .await
    .unwrap();

    let req: &Request = &s.received_requests().await.unwrap()[0];
    let facets = req
        .url
        .query_pairs()
        .find(|(k, _)| k == "facets")
        .unwrap()
        .1
        .into_owned();
    for f in [
        "project_type:mod",
        "categories:fabric",
        "versions:1.21.1",
        "client_side:required",
        "categories:optimization",
    ] {
        assert!(facets.contains(f), "{f} missing from {facets}");
    }
}

#[tokio::test]
async fn rate_limit_429_waits_and_retries_once() {
    let s = MockServer::start().await;
    Mock::given(path("/project/sodium"))
        .respond_with(ResponseTemplate::new(429).insert_header("x-ratelimit-reset", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(path("/project/sodium"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"AANobbMI","slug":"sodium","title":"Sodium","description":"d"}),
        ))
        .with_priority(2)
        .mount(&s)
        .await;
    let m = Modrinth::with_base(UA, &s.uri()).unwrap();
    let t = Instant::now();
    let p = m
        .project("sodium", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(p.id, "AANobbMI");
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "waited for the reset window"
    );
}

#[tokio::test]
async fn exhausted_budget_pauses_next_request_and_persistent_429_errors() {
    let s = MockServer::start().await;
    Mock::given(path("/version/a"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "1")
                .set_body_json(json!({"id":"a","project_id":"p","name":"n","version_number":"1"})),
        )
        .mount(&s)
        .await;
    Mock::given(path("/version/b"))
        .respond_with(ResponseTemplate::new(429).insert_header("x-ratelimit-reset", "1"))
        .mount(&s)
        .await;
    let m = Modrinth::with_base(UA, &s.uri()).unwrap();
    let c = CancellationToken::new();
    m.version("a", &c).await.unwrap();
    let t = Instant::now();
    let err = m.version("b", &c).await.unwrap_err();
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "paused until the window reset"
    );
    assert!(matches!(err, ModrinthError::RateLimited));
}

#[tokio::test]
async fn not_found_invalid_ids_and_hash_lookup() {
    let s = MockServer::start().await;
    Mock::given(path("/project/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .and(body_partial_json(
            json!({"algorithm":"sha512","hashes":["abc"]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"abc":{"id":"v","project_id":"p","name":"n","version_number":"1"}}),
        ))
        .mount(&s)
        .await;
    let m = Modrinth::with_base(UA, &s.uri()).unwrap();
    let c = CancellationToken::new();
    assert!(matches!(
        m.project("missing", &c).await,
        Err(ModrinthError::NotFound)
    ));
    assert!(
        matches!(
            m.project("../../etc", &c).await,
            Err(ModrinthError::NotFound)
        ),
        "rejected before any request"
    );
    assert_eq!(
        m.versions_by_hash(&["abc".into()], &c).await.unwrap()["abc"].project_id,
        "p"
    );
    assert!(m.versions_by_hash(&[], &c).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to the real Modrinth API and CDN"]
async fn real_api_plan_and_install_with_dependency() {
    let m = Modrinth::new(&nc_launcher::user_agent()).unwrap();
    let c = CancellationToken::new();
    let hits = m
        .search(
            &SearchQuery {
                query: "iris",
                loader: "fabric",
                minecraft: "1.21.1",
                category: None,
                sort: SortIndex::Relevance,
                offset: 0,
                limit: 5,
            },
            &c,
        )
        .await
        .unwrap();
    let iris = hits
        .hits
        .iter()
        .find(|h| h.slug.as_deref() == Some("iris"))
        .expect("iris in results");
    let versions = m
        .project_versions(&iris.project_id, "fabric", "1.21.1", &c)
        .await
        .unwrap();
    let root = pick_best(versions, "fabric", "1.21.1").unwrap();
    let p = plan::plan(&m, root, "fabric", "1.21.1", &HashSet::new(), &c)
        .await
        .unwrap();
    let projects: Vec<_> = p
        .install
        .iter()
        .map(|x| x.version.project_id.as_str())
        .collect();
    println!(
        "plan: {:?} missing {:?} conflicts {:?}",
        p.install
            .iter()
            .map(|x| &x.version.name)
            .collect::<Vec<_>>(),
        p.missing,
        p.conflicts
    );
    assert!(
        projects.contains(&"AANobbMI"),
        "Iris requires Sodium: {projects:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let dl = Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default()).unwrap();
    for item in &p.install {
        let f = mods::install_version(&dl, dir.path(), &item.version, &c)
            .await
            .unwrap();
        assert_eq!(f.sha512.len(), 128);
    }
    let files = mods::scan(dir.path()).unwrap();
    assert_eq!(files.len(), p.install.len());

    // Identify by hash and check for updates, as the Mods tab does.
    let hashes: Vec<String> = files.iter().map(|f| f.sha512.clone()).collect();
    let known = m.versions_by_hash(&hashes, &c).await.unwrap();
    assert_eq!(
        known.len(),
        files.len(),
        "every installed file is recognised by hash"
    );
    let latest = m
        .latest_by_hash(&hashes, "fabric", "1.21.1", &c)
        .await
        .unwrap();
    assert_eq!(latest.len(), files.len());
}
