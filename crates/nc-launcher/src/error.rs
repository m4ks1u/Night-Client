use std::path::PathBuf;

use nc_core::CoreError;

use crate::{auth::AuthError, download::DownloadError, fsutil::ExtractError};

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {what}: {message}")]
    Parse { what: String, message: String },
    #[error("Minecraft version `{0}` was not found in the official manifest")]
    VersionNotFound(String),
    #[error("version `{0}` is not installed")]
    NotInstalled(String),
    #[error("no official Java runtime `{0}` is available for Windows x64")]
    RuntimeUnavailable(String),
    #[error("Java problem: {0}")]
    JavaIncompatible(String),
    #[error("game files are missing ({0}); use Verify files to repair this version")]
    FilesMissing(String),
    #[error("secure credential storage failed: {0}")]
    Vault(String),
    #[error("invalid JVM arguments: {0}")]
    JvmArgs(#[from] nc_core::jvmargs::JvmArgError),
    #[error("operation cancelled")]
    Cancelled,
    #[error("failed to start the game: {0}")]
    Spawn(String),
}

impl LauncherError {
    /// Stable machine-readable code for the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Download(DownloadError::Cancelled) | Self::Cancelled => "cancelled",
            Self::Download(DownloadError::DiskSpace { .. }) => "disk_space",
            Self::Download(DownloadError::Integrity { .. }) => "integrity",
            Self::Download(_) => "network",
            Self::Core(_) => "metadata",
            Self::Auth(e) => e.code(),
            Self::Extract(_) => "archive",
            Self::Io { .. } => "io",
            Self::Parse { .. } => "metadata",
            Self::VersionNotFound(_) => "version_not_found",
            Self::NotInstalled(_) => "not_installed",
            Self::RuntimeUnavailable(_) => "runtime_unavailable",
            Self::Spawn(_) => "spawn",
            Self::JavaIncompatible(_) => "java_incompatible",
            Self::FilesMissing(_) => "files_missing",
            Self::Vault(_) => "vault",
            Self::JvmArgs(_) => "invalid_jvm_args",
        }
    }

    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }

    pub fn parse(what: impl Into<String>) -> impl FnOnce(serde_json::Error) -> Self {
        let what = what.into();
        move |e| Self::Parse {
            what,
            message: e.to_string(),
        }
    }
}
