//! Crash reports and the diagnostics bundle a user can attach to a bug report.
//!
//! Everything written here passes through [`logging::redact`] again, even
//! though log files are already redacted when written. The bundle contains
//! the diagnostics summary, recent log files and crash reports, and never the
//! database, the token vault or game files.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::logging;

const CRASH_PREFIX: &str = "crash-";
const SEEN_MARKER: &str = ".crash-seen";
const KEEP_CRASHES: usize = 10;
const MAX_LOG_FILES: usize = 5;
/// Per file: keep the last 4 MiB of a large log.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

fn unix(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write a crash report (`crash-<unix>-<pid>.txt`) to `log_dir` and keep only
/// the newest few. `details` is redacted before it is written.
pub fn write_crash(log_dir: &Path, version: &str, details: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(log_dir)?;
    let now = unix(SystemTime::now());
    let path = log_dir.join(format!("{CRASH_PREFIX}{now}-{}.txt", std::process::id()));
    let body = format!(
        "Night Client {version} crashed at unix time {now}.\n\n{}\n",
        logging::redact(details)
    );
    fs::write(&path, body)?;
    for old in crash_files(log_dir).into_iter().skip(KEEP_CRASHES) {
        let _ = fs::remove_file(old.0);
    }
    Ok(path)
}

/// Crash reports, newest first, with their modification time.
fn crash_files(log_dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut out: Vec<(PathBuf, SystemTime)> = fs::read_dir(log_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with(CRASH_PREFIX) && n.ends_with(".txt")
                })
                .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|e| std::cmp::Reverse(e.1));
    out
}

/// Number of crash reports newer than the last call, and the time of the
/// newest one. Marks them as seen, so each crash is announced once.
pub fn take_new_crashes(log_dir: &Path) -> (usize, Option<u64>) {
    let marker = log_dir.join(SEEN_MARKER);
    let seen = fs::metadata(&marker)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let fresh: Vec<_> = crash_files(log_dir)
        .into_iter()
        .filter(|(_, t)| *t > seen)
        .collect();
    let newest = fresh.first().map(|(_, t)| unix(*t));
    if !fresh.is_empty() {
        let _ = fs::write(&marker, b"");
    }
    (fresh.len(), newest)
}

/// What [`export_bundle`] put in the archive.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub log_files: usize,
    pub crash_reports: usize,
}

/// Write a zip at `out` with `diagnostics.json` (`summary_json`), the newest
/// log files and the crash reports from `log_dir`, every line redacted.
pub fn export_bundle(out: &Path, summary_json: &str, log_dir: &Path) -> io::Result<Exported> {
    let mut logs: Vec<(PathBuf, SystemTime)> = fs::read_dir(log_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with("night-client") && n.ends_with(".log")
                })
                .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
                .collect()
        })
        .unwrap_or_default();
    logs.sort_by_key(|e| std::cmp::Reverse(e.1));
    logs.truncate(MAX_LOG_FILES);
    let crashes = crash_files(log_dir);

    let tmp = out.with_extension("zip.tmp");
    let result = (|| -> io::Result<Exported> {
        let mut zip = zip::ZipWriter::new(fs::File::create(&tmp)?);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("diagnostics.json", opts)
            .map_err(io::Error::other)?;
        zip.write_all(logging::redact(summary_json).as_bytes())?;
        for (dir, list) in [("logs", &logs), ("crashes", &crashes)] {
            for (path, _) in list.iter() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                zip.start_file(format!("{dir}/{name}"), opts)
                    .map_err(io::Error::other)?;
                zip.write_all(redacted_tail(path)?.as_bytes())?;
            }
        }
        zip.finish().map_err(io::Error::other)?;
        Ok(Exported {
            log_files: logs.len(),
            crash_reports: crashes.len(),
        })
    })();
    match result {
        Ok(e) => {
            fs::rename(&tmp, out)?;
            Ok(e)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// The file's text (only the last [`MAX_FILE_BYTES`] of a large one),
/// redacted line by line.
fn redacted_tail(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    let start = bytes.len().saturating_sub(MAX_FILE_BYTES as usize);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let mut out = String::with_capacity(text.len());
    if start > 0 {
        out.push_str("[earlier lines omitted]\n");
    }
    for line in text.lines() {
        out.push_str(&logging::redact(line));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";

    #[test]
    fn crash_reports_are_redacted_announced_once_and_pruned() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(take_new_crashes(d.path()), (0, None));
        let p = write_crash(
            d.path(),
            "1.2.3",
            &format!("panicked: access_token={TOKEN}"),
        )
        .unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("Night Client 1.2.3 crashed"));
        assert!(!text.contains(TOKEN), "{text}");
        let (n, newest) = take_new_crashes(d.path());
        assert_eq!(n, 1);
        assert!(newest.is_some());
        assert_eq!(take_new_crashes(d.path()).0, 0, "announced only once");

        for i in 0..KEEP_CRASHES + 3 {
            fs::write(d.path().join(format!("{CRASH_PREFIX}{i}-1.txt")), "x").unwrap();
        }
        write_crash(d.path(), "1.2.3", "again").unwrap();
        assert_eq!(crash_files(d.path()).len(), KEEP_CRASHES);
    }

    #[test]
    fn bundle_holds_summary_logs_and_crashes_all_redacted() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("night-client.2026-11-08.log"),
            format!("line one\nBearer {TOKEN}\n"),
        )
        .unwrap();
        fs::write(d.path().join("unrelated.txt"), "not included").unwrap();
        fs::write(d.path().join("night-client.db"), "not included").unwrap();
        write_crash(d.path(), "1.0.0", "boom").unwrap();
        let out = d.path().join("bundle.zip");
        let e = export_bundle(
            &out,
            &format!(r#"{{"note":"refresh_token={TOKEN}"}}"#),
            d.path(),
        )
        .unwrap();
        assert_eq!(
            e,
            Exported {
                log_files: 1,
                crash_reports: 1
            }
        );

        let mut z = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names[0].starts_with("crashes/crash-"));
        assert_eq!(
            &names[1..],
            ["diagnostics.json", "logs/night-client.2026-11-08.log"]
        );
        for i in 0..z.len() {
            let mut entry = z.by_index(i).unwrap();
            let name = entry.name().unwrap().to_string();
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            assert!(!s.contains(TOKEN), "{name} leaked: {s}");
        }
        assert!(!d.path().join("bundle.zip.tmp").exists());
    }
}
