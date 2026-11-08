//! Structured file logging with secret redaction.
//!
//! Logs rotate daily into `<log_dir>/night-client.YYYY-MM-DD.log`, keeping
//! the newest 7 files. Writes go through a background thread so logging
//! never blocks the caller. Every line passes through [`redact`] as a second
//! line of defence: tokens are already wrapped in `Secret`, but a stray
//! `format!` of a raw response must still not reach disk.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use regex::Regex;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    EnvFilter, fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
};

pub const FILE_PREFIX: &str = "night-client";
pub const KEEP_FILES: usize = 7;

static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // JSON Web Tokens (Minecraft access tokens, XSTS/XBL tokens).
        (r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*", "<redacted-jwt>"),
        // Authorization headers.
        (r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{8,}", "${1}<redacted>"),
        // Query/form/JSON style secrets.
        (
            r#"(?i)((?:access_token|refresh_token|id_token|code_verifier|accessToken|identityToken|RpsTicket|client_secret)["']?\s*[=:]\s*["']?)[^"'&\s,}]{6,}"#,
            "${1}<redacted>",
        ),
        // `--accessToken <value>` in launch arguments (plain or Debug-formatted).
        (r#"(--accessToken"?,?\s+"?)[^"\s,\]]+"#, "${1}<redacted>"),
        // Opaque Microsoft tokens: very long unbroken base64url runs.
        (r"[A-Za-z0-9+/_=-]{300,}", "<redacted-blob>"),
    ]
    .into_iter()
    .map(|(p, r)| (Regex::new(p).expect("static redaction regex is valid"), r))
    .collect()
});

/// OAuth `code=` parameters, but not as part of another name (`error_code=`).
static OAUTH_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"((?:^|[^A-Za-z0-9_])code["']?\s*[=:]\s*["']?)([^"'&\s,}]+)"#)
        .expect("static redaction regex is valid")
});

/// Remove anything that looks like a credential from `line`.
pub fn redact(line: &str) -> String {
    let mut out = line.to_owned();
    for (re, rep) in PATTERNS.iter() {
        if re.is_match(&out) {
            out = re.replace_all(&out, *rep).into_owned();
        }
    }
    if OAUTH_CODE.is_match(&out) {
        // Authorization codes are long and contain dots, digits or capitals.
        // Short values (`code=0`) and plain lowercase identifiers such as our
        // own error codes (`auth_not_configured`) are kept for diagnostics.
        out = OAUTH_CODE
            .replace_all(&out, |c: &regex::Captures<'_>| {
                let value = &c[2];
                if value.len() < 6 || value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                    c[0].to_owned()
                } else {
                    format!("{}<redacted>", &c[1])
                }
            })
            .into_owned();
    }
    out
}

pub struct RedactingWriter<W: Write>(W);

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(redact(&text).as_bytes())?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[derive(Clone)]
struct MakeRedacting<M>(M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for MakeRedacting<M> {
    type Writer = RedactingWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter(self.0.make_writer())
    }
}

/// Install the global subscriber: redacted rolling file, plus stderr in
/// debug builds. Keep the returned guard alive for the whole process.
pub fn init(log_dir: &Path, default_filter: &str) -> io::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)?;
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(log_dir)
        .map_err(io::Error::other)?;
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(MakeRedacting(file));
    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    #[cfg(debug_assertions)]
    let registry =
        registry.with(tracing_subscriber::fmt::layer().with_writer(MakeRedacting(io::stderr)));
    registry.try_init().map_err(io::Error::other)?;
    Ok(guard)
}

/// Newest log file in `log_dir`, if any.
pub fn current_log_file(log_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(log_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(FILE_PREFIX))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// Last `max_lines` lines of a log file (reads at most the final 256 KiB).
pub fn tail(path: &Path, max_lines: usize) -> io::Result<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(256 * 1024);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably a partial line
    }
    let skip = lines.len().saturating_sub(max_lines);
    Ok(lines[skip..].iter().map(|l| redact(l)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secret_shapes() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcDEF123_-";
        let cases = [
            (format!("token {jwt} end"), jwt.to_string()),
            (
                "Authorization: Bearer abcdefgh12345678".into(),
                "abcdefgh12345678".into(),
            ),
            (
                "grant_type=refresh_token&refresh_token=M.C123_secret_value&x=1".into(),
                "M.C123_secret_value".into(),
            ),
            (
                r#"{"access_token":"EwBIA-opaque-value-123","expires_in":3600}"#.into(),
                "EwBIA-opaque-value-123".into(),
            ),
            (
                "code=M.R3_BAY.abc123def&state=xyz".into(),
                "M.R3_BAY.abc123def".into(),
            ),
            (
                r#"args: ["--accessToken", "tok-12345678", "--version"]"#.into(),
                "tok-12345678".into(),
            ),
            (
                "--accessToken tok-12345678 --version 1.21".into(),
                "tok-12345678".into(),
            ),
            (format!("blob {}", "A".repeat(400)), "A".repeat(300)),
        ];
        for (input, secret) in cases {
            let out = redact(&input);
            assert!(!out.contains(&secret), "leaked in: {out}");
            assert!(out.contains("redacted"), "{out}");
        }
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            r"INFO nc_launcher::install: installed 26.3 in C:\Users\x\AppData\Roaming\NightClient",
            "WARN retrying download url=https://resources.download.minecraft.net/67/672de5ad attempt=0",
            "exit code=0",
            "ERROR auth: xsts: unexpected HTTP 401",
            r#"WARN night_client_lib::commands::account: active account check failed code="auth_not_configured""#,
            "WARN sign-in failed error_code=auth_denied",
            "exit code=0",
        ] {
            assert_eq!(redact(line), line);
        }
    }

    #[test]
    fn writer_redacts_and_tail_reads_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("night-client.2026-01-01.log");
        {
            let mut w = RedactingWriter(std::fs::File::create(&p).unwrap());
            for i in 0..50 {
                writeln!(w, "line {i}").unwrap();
            }
            writeln!(w, "Bearer supersecretvalue123").unwrap();
        }
        let t = tail(&p, 3).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], "line 48");
        assert!(!t[2].contains("supersecret"));
        assert_eq!(current_log_file(dir.path()).unwrap(), p);
    }
}
