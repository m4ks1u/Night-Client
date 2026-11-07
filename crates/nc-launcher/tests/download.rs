//! Download engine behaviour against a local mock HTTP server.

#![allow(clippy::unwrap_used)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use nc_launcher::{
    CancellationToken,
    download::{DownloadError, DownloadTask, Downloader, DownloaderConfig, Progress, part_path},
};
use sha1::{Digest, Sha1};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

fn sha1_hex(b: &[u8]) -> String {
    hex::encode(Sha1::digest(b))
}

fn body(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn dl(cfg: DownloaderConfig) -> Downloader {
    Downloader::new("night-client-tests", cfg)
        .unwrap()
        .allow_insecure_for_tests()
}

fn fast() -> DownloaderConfig {
    DownloaderConfig {
        max_retries: 2,
        base_backoff: Duration::from_millis(10),
        read_timeout: Duration::from_millis(500),
        disk_reserve: 0,
        ..Default::default()
    }
}

fn task(server: &MockServer, p: &str, dest: &std::path::Path, data: &[u8]) -> DownloadTask {
    DownloadTask {
        url: format!("{}{p}", server.uri()),
        dest: dest.to_path_buf(),
        sha1: Some(sha1_hex(data)),
        size: Some(data.len() as u64),
    }
}

#[tokio::test]
async fn downloads_verifies_and_second_run_is_a_cache_hit() {
    let server = MockServer::start().await;
    let data = body(200_000);
    Mock::given(method("GET"))
        .and(path("/a.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("libs/a.jar");
    let t = task(&server, "/a.jar", &dest, &data);
    let d = dl(fast());

    let last = Arc::new(Mutex::new(Progress::default()));
    let l2 = last.clone();
    let s = d
        .fetch_all(vec![t.clone()], &CancellationToken::new(), move |p| {
            *l2.lock().unwrap() = p
        })
        .await
        .unwrap();
    assert_eq!(s.downloaded_files, 1);
    assert_eq!(s.downloaded_bytes, data.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert!(!part_path(&dest).exists());
    let p = *last.lock().unwrap();
    assert_eq!(
        (p.done_files, p.total_files, p.done_bytes),
        (1, 1, data.len() as u64)
    );

    // Second run: nothing downloaded (the mock's expect(1) verifies no request).
    let s = d
        .fetch_all(vec![t], &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        (s.downloaded_files, s.skipped_files, s.downloaded_bytes),
        (0, 1, 0)
    );
}

#[tokio::test]
async fn hash_mismatch_fails_and_leaves_no_file() {
    let server = MockServer::start().await;
    Mock::given(path("/bad"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"tampered".to_vec()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("bad.jar");
    let mut t = task(&server, "/bad", &dest, b"original");
    t.size = None;
    let err = dl(fast())
        .fetch_all(vec![t], &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::Integrity { .. }), "{err}");
    assert!(!dest.exists());
    assert!(!part_path(&dest).exists());
}

#[tokio::test]
async fn not_found_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let t = task(&server, "/missing", &dir.path().join("m"), b"x");
    let err = dl(fast())
        .fetch_all(vec![t], &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, DownloadError::Http { status: 404, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn server_errors_are_retried_then_succeed() {
    let server = MockServer::start().await;
    let data = body(1000);
    Mock::given(path("/flaky"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path("/flaky"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .with_priority(2)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f");
    dl(fast())
        .fetch_all(
            vec![task(&server, "/flaky", &dest, &data)],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), data);
}

#[tokio::test]
async fn persistent_server_error_gives_up_after_retries() {
    let server = MockServer::start().await;
    Mock::given(path("/down"))
        .respond_with(ResponseTemplate::new(500))
        .expect(3)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let err = dl(fast())
        .fetch_all(
            vec![task(&server, "/down", &dir.path().join("d"), b"x")],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::Http { status: 500, .. }));
}

/// Responds 206 with the requested suffix when a valid Range header is sent.
struct RangeResponder {
    data: Vec<u8>,
    seen_range: Arc<Mutex<Option<String>>>,
}

impl Respond for RangeResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let range = req
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *self.seen_range.lock().unwrap() = range.clone();
        match range
            .as_deref()
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.strip_suffix('-'))
        {
            Some(start) => {
                let start: usize = start.parse().unwrap();
                ResponseTemplate::new(206).set_body_bytes(self.data[start..].to_vec())
            }
            None => ResponseTemplate::new(200).set_body_bytes(self.data.clone()),
        }
    }
}

#[tokio::test]
async fn resumes_partial_download_with_range() {
    let server = MockServer::start().await;
    let data = body(100_000);
    let seen = Arc::new(Mutex::new(None));
    Mock::given(path("/big"))
        .respond_with(RangeResponder {
            data: data.clone(),
            seen_range: seen.clone(),
        })
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("big.bin");
    std::fs::write(part_path(&dest), &data[..40_000]).unwrap(); // simulated crash

    let s = dl(fast())
        .fetch_all(
            vec![task(&server, "/big", &dest, &data)],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().as_deref(), Some("bytes=40000-"));
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert_eq!(
        s.downloaded_bytes,
        data.len() as u64,
        "progress counts the resumed prefix once"
    );
}

#[tokio::test]
async fn complete_part_file_is_verified_without_a_request() {
    // Found in the real kill/resume test: the process died after the last
    // byte was written but before the rename.
    let server = MockServer::start().await;
    let data = body(8_489);
    Mock::given(path("/done"))
        .respond_with(ResponseTemplate::new(416))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("done.bin");
    std::fs::write(part_path(&dest), &data).unwrap();
    let s = dl(fast())
        .fetch_all(
            vec![task(&server, "/done", &dest, &data)],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(s.downloaded_files, 1);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert!(!part_path(&dest).exists());
}

#[tokio::test]
async fn server_ignoring_range_restarts_cleanly() {
    let server = MockServer::start().await;
    let data = body(50_000);
    Mock::given(path("/norange"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("n.bin");
    std::fs::write(part_path(&dest), &data[..10_000]).unwrap();
    dl(fast())
        .fetch_all(
            vec![task(&server, "/norange", &dest, &data)],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

#[tokio::test]
async fn corrupt_partial_file_is_discarded_after_integrity_failure() {
    let server = MockServer::start().await;
    let data = body(30_000);
    let seen = Arc::new(Mutex::new(None));
    Mock::given(path("/c"))
        .respond_with(RangeResponder {
            data: data.clone(),
            seen_range: seen,
        })
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("c.bin");
    std::fs::write(part_path(&dest), vec![0xFFu8; 5_000]).unwrap(); // garbage prefix
    dl(fast())
        .fetch_all(
            vec![task(&server, "/c", &dest, &data)],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(path("/huge"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body(5_000)))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("h");
    let t = DownloadTask {
        url: format!("{}/huge", server.uri()),
        dest: dest.clone(),
        sha1: None,
        size: Some(100),
    };
    let err = dl(fast())
        .fetch_all(vec![t], &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::Size { .. }), "{err}");
    assert!(!dest.exists());
}

#[tokio::test]
async fn cancel_stops_within_one_second() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body(10))
                .set_delay(Duration::from_secs(20)),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let tasks: Vec<_> = (0..4)
        .map(|i| {
            task(
                &server,
                "/slow",
                &dir.path().join(format!("s{i}")),
                &body(10),
            )
        })
        .collect();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        c2.cancel();
    });
    let mut cfg = fast();
    cfg.read_timeout = Duration::from_secs(60);
    let start = Instant::now();
    let err = dl(cfg).fetch_all(tasks, &cancel, |_| {}).await.unwrap_err();
    assert!(matches!(err, DownloadError::Cancelled), "{err}");
    assert!(
        start.elapsed() < Duration::from_millis(1200),
        "took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn stalled_server_times_out_and_is_retried() {
    let server = MockServer::start().await;
    let hits = Arc::new(AtomicUsize::new(0));
    struct Stall(Arc<AtomicUsize>);
    impl Respond for Stall {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            self.0.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200)
                .set_body_bytes(vec![1u8])
                .set_delay(Duration::from_secs(5))
        }
    }
    Mock::given(path("/stall"))
        .respond_with(Stall(hits.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = fast();
    cfg.read_timeout = Duration::from_millis(200);
    let err = dl(cfg)
        .fetch_all(
            vec![task(&server, "/stall", &dir.path().join("t"), &[1u8])],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::Network { .. }), "{err}");
    assert_eq!(hits.load(Ordering::SeqCst), 3, "1 attempt + 2 retries");
}

#[tokio::test]
async fn first_failure_cancels_remaining_downloads() {
    let server = MockServer::start().await;
    Mock::given(path("/404"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body(10))
                .set_delay(Duration::from_secs(20)),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let err = dl(fast())
        .fetch_all(
            vec![
                task(&server, "/slow", &dir.path().join("a"), &body(10)),
                task(&server, "/404", &dir.path().join("b"), b"x"),
            ],
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DownloadError::Http { status: 404, .. }),
        "root cause reported: {err}"
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn insecure_urls_are_refused_by_default() {
    let d = Downloader::new("t", DownloaderConfig::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let t = DownloadTask {
        url: "http://example.com/x".into(),
        dest: dir.path().join("x"),
        sha1: None,
        size: None,
    };
    assert!(matches!(
        d.fetch_all(vec![t], &CancellationToken::new(), |_| {})
            .await,
        Err(DownloadError::InsecureUrl(_))
    ));
}

#[tokio::test]
async fn insufficient_disk_space_is_reported_before_downloading() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let t = DownloadTask {
        url: format!("{}/x", server.uri()),
        dest: dir.path().join("x"),
        sha1: None,
        size: Some(u64::MAX / 4), // more than any disk
    };
    let err = dl(fast())
        .fetch_all(vec![t], &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::DiskSpace { .. }), "{err}");
}
