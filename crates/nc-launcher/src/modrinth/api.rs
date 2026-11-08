//! Modrinth API v2 client with a response cache and rate-limit handling.
//!
//! * Every request carries the launcher's identifying `User-Agent`, as
//!   Modrinth's API rules require.
//! * `X-Ratelimit-Remaining`/`-Reset` are honoured: when the budget is spent
//!   the client waits for the window to reset; a `429` waits and retries once.
//! * GET responses are cached in memory (bounded, time-limited) so browsing
//!   back and forth does not repeat requests.
//! * Responses are untrusted: file URLs must be HTTPS on `cdn.modrinth.com`,
//!   file names are validated, and long text is never rendered as HTML.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;

pub const API_BASE: &str = "https://api.modrinth.com/v2";
const CACHE_MAX: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ModrinthError {
    #[error("Modrinth could not be reached: {0}")]
    Network(String),
    #[error("Modrinth returned HTTP {0}")]
    Http(u16),
    #[error("not found on Modrinth")]
    NotFound,
    #[error("Modrinth is rate-limiting requests; try again in a minute")]
    RateLimited,
    #[error("unexpected response from Modrinth: {0}")]
    Parse(String),
    #[error("cancelled")]
    Cancelled,
}

impl ModrinthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Network(_) => "network",
            Self::Http(_) | Self::Parse(_) => "modrinth",
            Self::NotFound => "not_found",
            Self::RateLimited => "rate_limited",
            Self::Cancelled => "cancelled",
        }
    }
}

// ------------------------------------------------------------------ models --

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub offset: u32,
    pub limit: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub display_categories: Vec<String>,
    #[serde(default)]
    pub client_side: Option<String>,
    #[serde(default)]
    pub date_modified: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub client_side: Option<String>,
    #[serde(default)]
    pub project_type: String,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct License {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub version_type: String,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub date_published: String,
    #[serde(default)]
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

impl Version {
    /// The primary file, or the first file when none is marked primary.
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }

    pub fn supports(&self, loader: &str, minecraft: &str) -> bool {
        self.loaders.iter().any(|l| l == loader)
            && self.game_versions.iter().any(|g| g == minecraft)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VersionFile {
    pub hashes: Hashes,
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Hashes {
    pub sha1: String,
    pub sha512: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Dependency {
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    pub dependency_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Category {
    pub name: String,
    pub project_type: String,
    #[serde(default)]
    pub header: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortIndex {
    Relevance,
    Downloads,
    Follows,
    Newest,
    Updated,
}

impl SortIndex {
    fn as_str(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Downloads => "downloads",
            Self::Follows => "follows",
            Self::Newest => "newest",
            Self::Updated => "updated",
        }
    }
}

/// Parameters of a mod search.
#[derive(Debug, Clone, Copy)]
pub struct SearchQuery<'a> {
    pub query: &'a str,
    pub loader: &'a str,
    pub minecraft: &'a str,
    pub category: Option<&'a str>,
    pub sort: SortIndex,
    pub offset: u32,
    pub limit: u32,
}

/// Validate a Modrinth id or slug before it is put into a URL path.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Mod files may only come from Modrinth's CDN over HTTPS.
pub fn trusted_file_url(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some("cdn.modrinth.com"))
}

/// A file name from the API that is safe to create in a mods folder.
pub fn valid_mod_file_name(name: &str) -> bool {
    nc_core::paths::validate_segment(name).is_ok()
        && name.to_ascii_lowercase().ends_with(".jar")
        && name.len() <= 200
}

// ------------------------------------------------------------------ client --

/// (stored at, time to live, body)
type CacheEntry = (Instant, Duration, Vec<u8>);

pub struct Modrinth {
    http: reqwest::Client,
    base: String,
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// Do not send requests before this instant (rate-limit window reset).
    paused_until: Mutex<Option<Instant>>,
}

impl Modrinth {
    pub fn new(user_agent: &str) -> Result<Self, ModrinthError> {
        Self::with_base(user_agent, API_BASE)
    }

    pub fn with_base(user_agent: &str, base: &str) -> Result<Self, ModrinthError> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ModrinthError::Network(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            cache: Mutex::new(HashMap::new()),
            paused_until: Mutex::new(None),
        })
    }

    fn cached(&self, key: &str) -> Option<Vec<u8>> {
        let cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .get(key)
            .filter(|(at, ttl, _)| at.elapsed() < *ttl)
            .map(|(_, _, b)| b.clone())
    }

    fn store(&self, key: String, ttl: Duration, body: Vec<u8>) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= CACHE_MAX {
            // Evict expired entries first, then the oldest.
            cache.retain(|_, (at, ttl, _)| at.elapsed() < *ttl);
            if cache.len() >= CACHE_MAX
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, (at, _, _))| *at)
                    .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(key, (Instant::now(), ttl, body));
    }

    async fn wait_for_budget(&self, cancel: &CancellationToken) -> Result<(), ModrinthError> {
        let until = *self
            .paused_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = until
            && t > Instant::now()
        {
            tokio::select! {
                _ = tokio::time::sleep_until(t.into()) => {}
                _ = cancel.cancelled() => return Err(ModrinthError::Cancelled),
            }
        }
        Ok(())
    }

    fn note_limits(&self, headers: &reqwest::header::HeaderMap) {
        let num = |k: &str| {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        };
        if let (Some(0), Some(reset)) = (num("x-ratelimit-remaining"), num("x-ratelimit-reset")) {
            *self
                .paused_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(Instant::now() + Duration::from_secs(reset.min(60)));
        }
    }

    async fn request(
        &self,
        req: reqwest::RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ModrinthError> {
        for attempt in 0..2 {
            self.wait_for_budget(cancel).await?;
            let r = req
                .try_clone()
                .ok_or_else(|| ModrinthError::Parse("request not cloneable".into()))?;
            let resp = tokio::select! {
                r = r.send() => r.map_err(|e| ModrinthError::Network(e.without_url().to_string()))?,
                _ = cancel.cancelled() => return Err(ModrinthError::Cancelled),
            };
            self.note_limits(resp.headers());
            match resp.status().as_u16() {
                200..=299 => {
                    return resp
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| ModrinthError::Network(e.without_url().to_string()));
                }
                404 => return Err(ModrinthError::NotFound),
                429 if attempt == 0 => {
                    let reset = resp
                        .headers()
                        .get("x-ratelimit-reset")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(5)
                        .clamp(1, 60);
                    tracing::warn!(reset, "Modrinth rate limit hit, waiting");
                    *self
                        .paused_until
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(Instant::now() + Duration::from_secs(reset));
                }
                429 => return Err(ModrinthError::RateLimited),
                s => return Err(ModrinthError::Http(s)),
            }
        }
        Err(ModrinthError::RateLimited)
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path_and_query: &str,
        ttl: Duration,
        cancel: &CancellationToken,
    ) -> Result<T, ModrinthError> {
        let url = format!("{}{path_and_query}", self.base);
        let body = match self.cached(&url) {
            Some(b) => b,
            None => {
                let b = self.request(self.http.get(&url), cancel).await?;
                self.store(url, ttl, b.clone());
                b
            }
        };
        serde_json::from_slice(&body).map_err(|e| ModrinthError::Parse(e.to_string()))
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<T, ModrinthError> {
        let url = format!("{}{path}", self.base);
        let b = self
            .request(self.http.post(&url).json(body), cancel)
            .await?;
        serde_json::from_slice(&b).map_err(|e| ModrinthError::Parse(e.to_string()))
    }

    /// Search mods compatible with `loader` and `minecraft`.
    pub async fn search(
        &self,
        q: &SearchQuery<'_>,
        cancel: &CancellationToken,
    ) -> Result<SearchResponse, ModrinthError> {
        let SearchQuery {
            query,
            loader,
            minecraft,
            category,
            sort,
            offset,
            limit,
        } = *q;
        let mut facets = vec![
            vec!["project_type:mod".to_owned()],
            vec![format!("categories:{loader}")],
            vec![format!("versions:{minecraft}")],
            // Hide server-only mods: they do nothing in a client instance.
            vec![
                "client_side:required".to_owned(),
                "client_side:optional".to_owned(),
            ],
        ];
        if let Some(c) = category {
            facets.push(vec![format!("categories:{c}")]);
        }
        let facets =
            serde_json::to_string(&facets).map_err(|e| ModrinthError::Parse(e.to_string()))?;
        let mut u = url::Url::parse(&format!("{}/search", self.base))
            .map_err(|e| ModrinthError::Parse(e.to_string()))?;
        u.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("facets", &facets)
            .append_pair("index", sort.as_str())
            .append_pair("offset", &offset.min(10_000).to_string())
            .append_pair("limit", &limit.clamp(1, 50).to_string());
        let pq = format!("/search?{}", u.query().unwrap_or(""));
        self.get(&pq, Duration::from_secs(300), cancel).await
    }

    pub async fn project(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<Project, ModrinthError> {
        if !valid_id(id) {
            return Err(ModrinthError::NotFound);
        }
        self.get(&format!("/project/{id}"), Duration::from_secs(1800), cancel)
            .await
    }

    pub async fn version(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<Version, ModrinthError> {
        if !valid_id(id) {
            return Err(ModrinthError::NotFound);
        }
        self.get(&format!("/version/{id}"), Duration::from_secs(3600), cancel)
            .await
    }

    /// Versions of a project for a loader and game version, newest first.
    pub async fn project_versions(
        &self,
        project: &str,
        loader: &str,
        minecraft: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<Version>, ModrinthError> {
        if !valid_id(project) {
            return Err(ModrinthError::NotFound);
        }
        let loaders =
            serde_json::to_string(&[loader]).map_err(|e| ModrinthError::Parse(e.to_string()))?;
        let games =
            serde_json::to_string(&[minecraft]).map_err(|e| ModrinthError::Parse(e.to_string()))?;
        let mut u = url::Url::parse(&format!("{}/project/{project}/version", self.base))
            .map_err(|e| ModrinthError::Parse(e.to_string()))?;
        u.query_pairs_mut()
            .append_pair("loaders", &loaders)
            .append_pair("game_versions", &games);
        let pq = format!("/project/{project}/version?{}", u.query().unwrap_or(""));
        self.get(&pq, Duration::from_secs(600), cancel).await
    }

    /// Identify files by SHA-512: `{ hash: version }` for known files.
    pub async fn versions_by_hash(
        &self,
        sha512: &[String],
        cancel: &CancellationToken,
    ) -> Result<HashMap<String, Version>, ModrinthError> {
        if sha512.is_empty() {
            return Ok(HashMap::new());
        }
        self.post(
            "/version_files",
            &serde_json::json!({ "hashes": sha512, "algorithm": "sha512" }),
            cancel,
        )
        .await
    }

    /// Latest compatible version for each file hash: `{ hash: version }`.
    pub async fn latest_by_hash(
        &self,
        sha512: &[String],
        loader: &str,
        minecraft: &str,
        cancel: &CancellationToken,
    ) -> Result<HashMap<String, Version>, ModrinthError> {
        if sha512.is_empty() {
            return Ok(HashMap::new());
        }
        let body = serde_json::json!({ "hashes": sha512, "algorithm": "sha512", "loaders": [loader], "game_versions": [minecraft] });
        self.post("/version_files/update", &body, cancel).await
    }

    pub async fn categories(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<Category>, ModrinthError> {
        let all: Vec<Category> = self
            .get("/tag/category", Duration::from_secs(86_400), cancel)
            .await?;
        Ok(all
            .into_iter()
            .filter(|c| c.project_type == "mod" && c.header == "categories")
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_input_checks() {
        assert!(valid_id("AANobbMI") && valid_id("sodium") && valid_id("fabric-api"));
        assert!(!valid_id("../x") && !valid_id("a/b") && !valid_id("") && !valid_id("a?b=c"));
        assert!(trusted_file_url(
            "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"
        ));
        for bad in [
            "http://cdn.modrinth.com/x.jar",
            "https://evil.com/x.jar",
            "https://cdn.modrinth.com.evil.com/x.jar",
            "file:///C:/x.jar",
        ] {
            assert!(!trusted_file_url(bad), "{bad}");
        }
        assert!(valid_mod_file_name("sodium-fabric-0.8.13+mc1.21.1.jar"));
        for bad in [
            "../mods/x.jar",
            "x.exe",
            "x.jar:ads",
            "con.jar",
            "a/b.jar",
            "",
        ] {
            assert!(!valid_mod_file_name(bad), "{bad}");
        }
    }
}
