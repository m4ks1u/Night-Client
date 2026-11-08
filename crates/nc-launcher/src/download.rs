//! Bounded, cancellable, resumable download engine with integrity checks.
//!
//! * Files are written to `<dest>.part` and renamed into place only after the
//!   size and SHA-1 have been verified, so a destination file is either
//!   absent or complete.
//! * Interrupted `.part` files are resumed with an HTTP `Range` request when
//!   the server supports it, otherwise downloaded again.
//! * Already-valid files are skipped, which makes shared `libraries/` and
//!   `assets/` directories act as a cache across instances.
//! * Transient failures (network errors, 408/429/5xx, integrity mismatches)
//!   are retried with exponential backoff and jitter.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures::{StreamExt, stream};
use reqwest::{StatusCode, header};
use serde::Serialize;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{confined::Confined, fsutil};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTask {
    pub url: String,
    pub dest: PathBuf,
    /// Expected lower- or upper-case hex SHA-1.
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// When set, `dest` is a path from untrusted data inside this root: the
    /// `.part` file and destination are checked through it (no link on the
    /// way, folder still inside the root) before every open, rename and
    /// cleanup, and a cleanup whose path is no longer trusted is skipped.
    pub root: Option<Confined>,
}

impl DownloadTask {
    /// Fail unless `path` is still safe to open or rename (only with a root).
    fn check(&self, path: &Path) -> Result<(), DownloadError> {
        match &self.root {
            Some(r) => r.check(path).map_err(io_err(path)),
            None => Ok(()),
        }
    }

    /// Remove a `.part` file, through the root's safe cleanup when set.
    async fn remove_part(&self, part: &Path) {
        match &self.root {
            Some(r) => {
                r.remove_temp(part);
            }
            None => {
                let _ = tokio::fs::remove_file(part).await;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloaderConfig {
    pub concurrency: usize,
    pub max_retries: u32,
    pub connect_timeout: Duration,
    /// Maximum time without receiving any bytes.
    pub read_timeout: Duration,
    pub base_backoff: Duration,
    /// Free space to keep on the volume in addition to the download size.
    pub disk_reserve: u64,
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        Self {
            concurrency: 8,
            max_retries: 4,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            base_backoff: Duration::from_millis(500),
            disk_reserve: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub total_files: u64,
    pub done_files: u64,
    pub skipped_files: u64,
    /// Sum of known sizes of files that need downloading.
    pub total_bytes: u64,
    pub done_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub downloaded_files: u64,
    pub skipped_files: u64,
    pub downloaded_bytes: u64,
    /// Destinations that were (re-)downloaded, i.e. missing or damaged before.
    #[serde(skip)]
    pub fetched: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download cancelled")]
    Cancelled,
    #[error("{url}: server returned HTTP {status}")]
    Http { url: String, status: u16 },
    #[error("{url}: network error: {message}")]
    Network { url: String, message: String },
    #[error("{url}: integrity check failed (expected {expected}, got {actual})")]
    Integrity {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("{url}: size mismatch (expected {expected} bytes, got {actual})")]
    Size {
        url: String,
        expected: u64,
        actual: u64,
    },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("not enough disk space: need {needed} bytes, {available} available")]
    DiskSpace { needed: u64, available: u64 },
    #[error("refusing non-HTTPS URL {0}")]
    InsecureUrl(String),
    #[error("could not initialise HTTP client: {0}")]
    Client(String),
}

impl DownloadError {
    fn retryable(&self) -> bool {
        match self {
            Self::Network { .. } | Self::Integrity { .. } | Self::Size { .. } => true,
            Self::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> DownloadError + '_ {
    move |source| DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    cfg: Arc<DownloaderConfig>,
    allow_http: bool,
}

#[derive(Default)]
struct Counters {
    done_files: AtomicU64,
    done_bytes: AtomicU64,
    fetched: std::sync::Mutex<Vec<PathBuf>>,
}

impl Downloader {
    pub fn new(user_agent: &str, cfg: DownloaderConfig) -> Result<Self, DownloadError> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(cfg.connect_timeout)
            .read_timeout(cfg.read_timeout)
            .https_only(false) // enforced per-request below so tests can use http://127.0.0.1
            .build()
            .map_err(|e| DownloadError::Client(e.to_string()))?;
        Ok(Self {
            client,
            cfg: Arc::new(cfg),
            allow_http: false,
        })
    }

    /// Permit plain-HTTP URLs. Only for tests against a local mock server.
    #[doc(hidden)]
    pub fn allow_insecure_for_tests(mut self) -> Self {
        self.allow_http = true;
        self
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn check_url(&self, url: &str) -> Result<(), DownloadError> {
        if url.starts_with("https://") || (self.allow_http && url.starts_with("http://")) {
            Ok(())
        } else {
            Err(DownloadError::InsecureUrl(url.to_owned()))
        }
    }

    /// Fetch a small document into memory, with the same retry policy.
    pub async fn get_bytes(
        &self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, DownloadError> {
        self.check_url(url)?;
        let mut attempt = 0;
        loop {
            let result = async {
                let resp = tokio::select! {
                    r = self.client.get(url).send() => r.map_err(|e| net(url, &e))?,
                    _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
                };
                let status = resp.status();
                if !status.is_success() {
                    return Err(DownloadError::Http {
                        url: url.into(),
                        status: status.as_u16(),
                    });
                }
                tokio::select! {
                    b = resp.bytes() => b.map(|b| b.to_vec()).map_err(|e| net(url, &e)),
                    _ = cancel.cancelled() => Err(DownloadError::Cancelled),
                }
            }
            .await;
            match result {
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    self.backoff(attempt, cancel).await?;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// Download every task. Fails fast: the first non-retryable error cancels
    /// the remaining transfers. `on_progress` is called at most every 100 ms
    /// and once at the end.
    pub async fn fetch_all(
        &self,
        tasks: Vec<DownloadTask>,
        cancel: &CancellationToken,
        on_progress: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Result<Summary, DownloadError> {
        for t in &tasks {
            self.check_url(&t.url)?;
        }

        // De-duplicate by destination.
        let mut seen = HashSet::new();
        let tasks: Vec<DownloadTask> = tasks
            .into_iter()
            .filter(|t| seen.insert(t.dest.clone()))
            .collect();
        let total_files = tasks.len() as u64;

        // Verify existing files off the async runtime (hashing is CPU-bound).
        let checks = stream::iter(tasks.into_iter().map(|t| {
            tokio::task::spawn_blocking(move || {
                let ok = fsutil::is_valid(&t.dest, t.sha1.as_deref(), t.size);
                (t, ok)
            })
        }))
        .buffer_unordered(num_threads())
        .collect::<Vec<_>>()
        .await;
        let mut pending = Vec::new();
        let mut skipped = 0u64;
        for r in checks {
            let (t, ok) = r.map_err(|e| DownloadError::Client(e.to_string()))?;
            if ok {
                skipped += 1;
            } else {
                pending.push(t);
            }
        }
        if cancel.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }

        let total_bytes: u64 = pending.iter().filter_map(|t| t.size).sum();
        if let Some(first) = pending.first() {
            let available = fsutil::free_space(&first.dest).map_err(io_err(&first.dest))?;
            let needed = total_bytes + self.cfg.disk_reserve;
            if available < needed {
                return Err(DownloadError::DiskSpace { needed, available });
            }
        }

        let counters = Arc::new(Counters::default());
        let snapshot = {
            let c = counters.clone();
            move || Progress {
                total_files,
                done_files: skipped + c.done_files.load(Ordering::Relaxed),
                skipped_files: skipped,
                total_bytes,
                done_bytes: c.done_bytes.load(Ordering::Relaxed),
            }
        };

        let on_progress = Arc::new(on_progress);
        let run_token = cancel.child_token();
        let ticker = {
            let (p, s, tok) = (on_progress.clone(), snapshot.clone(), run_token.clone());
            tokio::spawn(async move {
                let mut iv = tokio::time::interval(Duration::from_millis(100));
                loop {
                    tokio::select! {
                        _ = iv.tick() => p(s()),
                        _ = tok.cancelled() => break,
                    }
                }
            })
        };

        let results = stream::iter(pending.into_iter().map(|task| {
            let (this, counters, tok) = (self.clone(), counters.clone(), run_token.clone());
            async move {
                let r = this.download_with_retry(&task, &tok, &counters).await;
                if r.is_ok() {
                    counters.done_files.fetch_add(1, Ordering::Relaxed);
                    counters
                        .fetched
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(task.dest.clone());
                } else {
                    tok.cancel(); // fail fast
                }
                r
            }
        }))
        .buffer_unordered(self.cfg.concurrency.max(1))
        .collect::<Vec<_>>()
        .await;

        run_token.cancel();
        let _ = ticker.await;
        on_progress(snapshot());

        if cancel.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        // Report the root cause, not the cancellations it triggered.
        let mut cancelled = false;
        for r in results {
            match r {
                Err(DownloadError::Cancelled) => cancelled = true,
                Err(e) => return Err(e),
                Ok(()) => {}
            }
        }
        if cancelled {
            return Err(DownloadError::Cancelled);
        }
        Ok(Summary {
            downloaded_files: counters.done_files.load(Ordering::Relaxed),
            skipped_files: skipped,
            downloaded_bytes: counters.done_bytes.load(Ordering::Relaxed),
            fetched: std::mem::take(
                &mut *counters
                    .fetched
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
        })
    }

    async fn download_with_retry(
        &self,
        task: &DownloadTask,
        cancel: &CancellationToken,
        counters: &Counters,
    ) -> Result<(), DownloadError> {
        let mut attempt = 0;
        loop {
            let mut counted = 0u64;
            let r = self
                .download_once(task, cancel, counters, &mut counted)
                .await;
            match r {
                Ok(()) => return Ok(()),
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    tracing::warn!(url = %task.url, attempt, error = %e, "retrying download");
                    counters.done_bytes.fetch_sub(counted, Ordering::Relaxed);
                    self.backoff(attempt, cancel).await?;
                    attempt += 1;
                }
                Err(e) => {
                    counters.done_bytes.fetch_sub(counted, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
    }

    async fn backoff(&self, attempt: u32, cancel: &CancellationToken) -> Result<(), DownloadError> {
        let base = self.cfg.base_backoff.saturating_mul(1 << attempt.min(4));
        let jitter_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()))
            .unwrap_or(0)
            % (self.cfg.base_backoff.as_millis() as u64 + 1);
        let delay = base.min(Duration::from_secs(8)) + Duration::from_millis(jitter_ms);
        tokio::select! {
            _ = tokio::time::sleep(delay) => Ok(()),
            _ = cancel.cancelled() => Err(DownloadError::Cancelled),
        }
    }

    async fn download_once(
        &self,
        task: &DownloadTask,
        cancel: &CancellationToken,
        counters: &Counters,
        counted: &mut u64,
    ) -> Result<(), DownloadError> {
        let dest = &task.dest;
        let parent = dest.parent().ok_or_else(|| DownloadError::Io {
            path: dest.clone(),
            source: std::io::Error::other("destination has no parent"),
        })?;
        match &task.root {
            Some(r) => {
                let rel = r.rel_of(dest).map_err(io_err(dest))?;
                r.create_parent(&rel).map_err(io_err(dest))?;
            }
            None => tokio::fs::create_dir_all(parent)
                .await
                .map_err(io_err(parent))?,
        }
        let part = part_path(dest);
        task.check(&part)?;

        let mut hasher = Sha1::new();
        let mut existing = tokio::fs::metadata(&part)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if task.size.is_some_and(|s| existing > s) {
            existing = 0;
        }
        if existing > 0 {
            hash_prefix(&part, existing, &mut hasher)
                .await
                .map_err(io_err(&part))?;
        }
        if existing > 0 && task.size == Some(existing) {
            // Interrupted after the last byte but before the rename: verify only.
            counters.done_bytes.fetch_add(existing, Ordering::Relaxed);
            *counted += existing;
            return finalize(task, &part, hasher).await;
        }

        let mut req = self.client.get(&task.url);
        if existing > 0 {
            req = req.header(header::RANGE, format!("bytes={existing}-"));
        }
        let resp = tokio::select! {
            r = req.send() => r.map_err(|e| net(&task.url, &e))?,
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
        };

        let append = match resp.status() {
            StatusCode::PARTIAL_CONTENT if existing > 0 => true,
            s if s.is_success() => {
                if existing > 0 {
                    hasher = Sha1::new(); // server ignored Range: start over
                }
                existing = 0;
                false
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                task.remove_part(&part).await;
                return Err(DownloadError::Size {
                    url: task.url.clone(),
                    expected: task.size.unwrap_or(0),
                    actual: existing,
                });
            }
            s => {
                return Err(DownloadError::Http {
                    url: task.url.clone(),
                    status: s.as_u16(),
                });
            }
        };

        task.check(&part)?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .await
            .map_err(io_err(&part))?;

        counters.done_bytes.fetch_add(existing, Ordering::Relaxed);
        *counted += existing;
        let mut written = existing;
        let mut body = resp.bytes_stream();
        loop {
            let chunk = tokio::select! {
                c = body.next() => c,
                _ = cancel.cancelled() => {
                    let _ = file.flush().await;
                    return Err(DownloadError::Cancelled); // keep .part for resume
                }
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| net(&task.url, &e))?;
            file.write_all(&chunk).await.map_err(io_err(&part))?;
            hasher.update(&chunk);
            written += chunk.len() as u64;
            counters
                .done_bytes
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
            *counted += chunk.len() as u64;
            if let Some(expected) = task.size
                && written > expected
            {
                drop(file);
                task.remove_part(&part).await;
                return Err(DownloadError::Size {
                    url: task.url.clone(),
                    expected,
                    actual: written,
                });
            }
        }
        file.flush().await.map_err(io_err(&part))?;
        file.sync_all().await.map_err(io_err(&part))?;
        drop(file);

        if let Some(expected) = task.size
            && written != expected
        {
            // Truncated body: keep the part file so the retry can resume.
            return Err(DownloadError::Size {
                url: task.url.clone(),
                expected,
                actual: written,
            });
        }
        finalize(task, &part, hasher).await
    }
}

/// Verify the SHA-1 of a complete `.part` file and atomically move it into place.
async fn finalize(task: &DownloadTask, part: &Path, hasher: Sha1) -> Result<(), DownloadError> {
    let dest = &task.dest;
    {
        if let Some(expected) = &task.sha1 {
            let actual = hex::encode(hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                task.remove_part(part).await;
                return Err(DownloadError::Integrity {
                    url: task.url.clone(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        task.check(part)?;
        task.check(dest)?;
        tokio::fs::rename(part, dest).await.map_err(io_err(dest))?;
        Ok(())
    }
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

async fn hash_prefix(path: &Path, len: u64, hasher: &mut Sha1) -> std::io::Result<()> {
    let mut f = tokio::fs::File::open(path).await?.take(len);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        hasher.update(&buf[..n]);
    }
}

fn net(url: &str, e: &reqwest::Error) -> DownloadError {
    // reqwest error strings can include the URL; URLs we fetch carry no secrets.
    DownloadError::Network {
        url: url.to_owned(),
        message: e.to_string(),
    }
}

fn num_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}
