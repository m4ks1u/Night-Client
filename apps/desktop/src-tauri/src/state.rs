use std::sync::Mutex;

use nc_launcher::{
    CancellationToken,
    auth::{AuthConfig, Client, MinecraftSession},
    download::{Downloader, DownloaderConfig},
    layout::Layout,
    meta::Meta,
};

pub struct AppState {
    pub meta: Meta,
    pub auth: Option<Client>,
    /// In-memory only for Stage 0. Stage 3 persists the refresh token with DPAPI.
    pub session: Mutex<Option<MinecraftSession>>,
    pub install: Mutex<Option<CancellationToken>>,
    pub sign_in: Mutex<Option<CancellationToken>>,
    pub game: Mutex<Option<GameHandle>>,
}

pub struct GameHandle {
    pub stop: CancellationToken,
    pub version: String,
}

impl AppState {
    pub fn new() -> Result<Self, nc_launcher::download::DownloadError> {
        let layout = Layout::new(Layout::default_root());
        let dl = Downloader::new(&nc_launcher::user_agent(), DownloaderConfig::default())?;
        Ok(Self {
            meta: Meta::new(dl, layout),
            auth: client_id()
                .and_then(|id| Client::new(AuthConfig::new(id), &nc_launcher::user_agent()).ok()),
            session: Mutex::new(None),
            install: Mutex::new(None),
            sign_in: Mutex::new(None),
            game: Mutex::new(None),
        })
    }
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
/// data (an Option) is still valid.
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
