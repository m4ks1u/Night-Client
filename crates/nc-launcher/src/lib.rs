//! Night Client launcher services.
//!
//! Everything here is UI-agnostic: the Tauri layer only wires these services
//! to commands and events. Long-running operations are async, cancellable
//! through a [`tokio_util::sync::CancellationToken`] and report progress
//! through plain callbacks.

pub mod accounts;
pub mod auth;
pub mod download;
pub mod error;
pub mod fsutil;
pub mod install;
pub mod instance;
pub mod java;
pub mod javadetect;
#[cfg(windows)]
pub mod javareg;
pub mod layout;
pub mod loaders;
pub mod logging;
pub mod meta;
pub mod modrinth;
pub mod process;
pub mod queue;
pub mod relocate;
pub mod report;
pub mod sys;
pub mod vault;

pub use error::LauncherError;
pub use tokio_util::sync::CancellationToken;

/// `User-Agent` sent to every service, as required by Modrinth's API rules
/// and good practice for Mojang's CDN.
pub fn user_agent() -> String {
    format!(
        "NightClient/{} (+https://github.com/m4ks1u/Night-Client)",
        env!("CARGO_PKG_VERSION")
    )
}
