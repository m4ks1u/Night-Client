use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
};

use nc_launcher::{
    CancellationToken,
    auth::{AuthConfig, Client, MinecraftSession},
    download::{Downloader, DownloaderConfig},
    layout::Layout,
    meta::Meta,
    queue::Queue,
    vault::{DpapiVault, TokenVault},
};
use nc_store::{AppSettings, Store};

pub struct AppState {
    /// Game data (versions, libraries, assets, runtimes, instances, cache).
    pub layout: Layout,
    /// `%APPDATA%\NightClient`: the database and the saved data location
    /// always live here, wherever the game data is.
    pub home: PathBuf,
    /// Set while the game data is being moved; installs, launches and
    /// instance or mod changes are refused meanwhile.
    pub relocating: AtomicBool,
    /// Whether this run's WebView draws with the GPU (the setting is read once, at startup).
    pub gpu_active: bool,
    pub log_dir: PathBuf,
    /// `None` when the database could not be opened (e.g. created by a newer
    /// version); the UI then shows `startup_error` instead of crashing.
    pub store: Option<Arc<Store>>,
    pub startup_error: Option<String>,
    /// Rebuilt when download settings change; installs clone it at start.
    meta: RwLock<Meta>,
    pub auth: Option<Client>,
    /// DPAPI-protected refresh tokens, keyed by profile UUID.
    pub vault: Arc<dyn TokenVault>,
    /// Live Minecraft sessions (access tokens), keyed by profile UUID. Memory only.
    pub sessions: Mutex<HashMap<String, MinecraftSession>>,
    /// Modrinth API client (response cache and rate-limit state live here).
    pub modrinth: nc_launcher::modrinth::Modrinth,
    /// Serialises token renewal so two launches never race a rotating refresh token.
    pub refresh_lock: tokio::sync::Mutex<()>,
    pub sign_in: Mutex<Option<CancellationToken>>,
    /// Download queue (one active job, FIFO pending); the worker in
    /// `commands::downloads` drains it.
    pub queue: Mutex<Queue>,
    pub queue_wake: tokio::sync::Notify,
    /// Running games keyed by instance id.
    pub games: Mutex<HashMap<String, GameHandle>>,
}

pub struct GameHandle {
    pub stop: CancellationToken,
    pub started_at: i64,
}

impl AppState {
    pub fn new(home: PathBuf, layout: Layout, log_dir: PathBuf) -> Self {
        let (store, startup_error) = match Store::open(&home.join("night-client.db")) {
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
            home,
            relocating: AtomicBool::new(false),
            gpu_active: settings.gpu_acceleration,
            log_dir,
            store,
            startup_error,
            auth: client_id()
                .and_then(|id| Client::new(AuthConfig::new(id), &nc_launcher::user_agent()).ok()),
            vault: Arc::new(DpapiVault::new(vault_dir())),
            sessions: Mutex::new(HashMap::new()),
            refresh_lock: tokio::sync::Mutex::new(()),
            modrinth: nc_launcher::modrinth::Modrinth::new(&nc_launcher::user_agent())
                .expect("HTTP client initialises"),
            sign_in: Mutex::new(None),
            queue: Mutex::new(Queue::default()),
            queue_wake: tokio::sync::Notify::new(),
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

/// `%LOCALAPPDATA%\NightClient\vault`: machine-local, never roamed.
fn vault_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(Layout::default_root)
        .join("NightClient")
        .join("vault")
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
