use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

use nc_launcher::{
    CancellationToken,
    auth::{AuthConfig, Client, MinecraftSession},
    download::{Downloader, DownloaderConfig},
    layout::Layout,
    meta::Meta,
};
use nc_store::{AppSettings, Store};

pub struct AppState {
    pub layout: Layout,
    pub log_dir: PathBuf,
    /// `None` when the database could not be opened (e.g. created by a newer
    /// version); the UI then shows `startup_error` instead of crashing.
    pub store: Option<Arc<Store>>,
    pub startup_error: Option<String>,
    /// Rebuilt when download settings change; installs clone it at start.
    meta: RwLock<Meta>,
    pub auth: Option<Client>,
    /// In memory only until the DPAPI vault lands in Stage 3.
    pub session: Mutex<Option<MinecraftSession>>,
    pub sign_in: Mutex<Option<CancellationToken>>,
    /// The one running installation: (version id, cancel token).
    pub install: Mutex<Option<(String, CancellationToken)>>,
    /// Running games keyed by instance id.
    pub games: Mutex<HashMap<String, GameHandle>>,
}

pub struct GameHandle {
    pub stop: CancellationToken,
    pub started_at: i64,
}

impl AppState {
    pub fn new(layout: Layout, log_dir: PathBuf) -> Self {
        let (store, startup_error) = match Store::open(&layout.root().join("night-client.db")) {
            Ok(s) => (Some(Arc::new(s)), None),
            Err(e) => {
                tracing::error!(error = %e, "database unavailable");
                (None, Some(e.to_string()))
            }
        };
        let settings = store.as_ref().map(|s| s.app_settings()).unwrap_or_default();
        Self {
            meta: RwLock::new(build_meta(&layout, &settings)),
            layout,
            log_dir,
            store,
            startup_error,
            auth: client_id()
                .and_then(|id| Client::new(AuthConfig::new(id), &nc_launcher::user_agent()).ok()),
            session: Mutex::new(None),
            sign_in: Mutex::new(None),
            install: Mutex::new(None),
            games: Mutex::new(HashMap::new()),
        }
    }

    pub fn meta(&self) -> Meta {
        self.meta
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn apply_settings(&self, settings: &AppSettings) {
        *self
            .meta
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            build_meta(&self.layout, settings);
    }
}

fn build_meta(layout: &Layout, s: &AppSettings) -> Meta {
    let cfg = DownloaderConfig {
        concurrency: usize::from(s.download_concurrency.clamp(1, 16)),
        ..Default::default()
    };
    // Building a reqwest client only fails if the TLS backend cannot load,
    // which would make the launcher unusable anyway.
    let dl = Downloader::new(&nc_launcher::user_agent(), cfg).expect("HTTP client initialises");
    Meta::new(dl, layout.clone())
}

/// Azure application (client) ID: runtime env var overrides the build-time
/// value. It is a public identifier, not a secret, but it is kept out of the
/// source tree so forks register their own.
pub fn client_id() -> Option<String> {
    std::env::var("NIGHT_CLIENT_MSA_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("NIGHT_CLIENT_MSA_CLIENT_ID").map(str::to_owned))
        .filter(|s| is_guid(s))
}

fn is_guid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// Lock helper: a poisoned mutex only means another command panicked; the
/// data is still valid.
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn guid_validation() {
        assert!(super::is_guid("00000000-0000-0000-0000-000000000000"));
        assert!(!super::is_guid("not-a-guid"));
        assert!(!super::is_guid("00000000-0000-0000-0000-00000000000g"));
    }
}
