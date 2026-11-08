//! Installing a Modrinth modpack (`.mrpack`) into an instance folder.
//!
//! 1. [`read_pack`]: read `modrinth.index.json` (size-capped) and validate it
//!    with `nc_core::mrpack` before anything is written.
//! 2. [`download_files`]: every file through the verified download engine
//!    (HTTPS, allowed hosts, SHA-1 and size), then SHA-512 as well.
//! 3. [`extract_overrides`]: copy `overrides/` then `client-overrides/` into
//!    the instance, with the same path rules, no symlinks and a size cap.
//!
//! Every write goes through [`Confined`]: a junction or symlink already in
//! the instance folder is never followed.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use nc_core::mrpack::{
    Index, MAX_INDEX_BYTES, Pack, PackFile, instance_path_allowed, is_reserved_dir, override_target,
};
use tokio_util::sync::CancellationToken;

use super::{
    api::{VersionFile, trusted_file_url},
    mods::sha512_file,
};
use crate::{
    LauncherError,
    confined::Confined,
    download::{DownloadTask, Downloader, Progress, Summary},
    fsutil::ExtractError,
};

/// SHA-512 (lower-case hex) of a pack file, to look it up on Modrinth.
pub fn sha512_of(path: &Path) -> std::io::Result<String> {
    sha512_file(path)
}

/// Overrides (configs, options, small resources) larger than this in total
/// are refused: it stops a zip bomb, not a real pack.
pub const MAX_OVERRIDES_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn open(
    archive: &Path,
) -> Result<zip::ZipArchive<std::io::BufReader<std::fs::File>>, LauncherError> {
    let f = std::fs::File::open(archive).map_err(LauncherError::io(archive))?;
    zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| LauncherError::Extract(e.into()))
}

/// Read and validate a pack's index (call off the async runtime).
pub fn read_pack(archive: &Path) -> Result<Pack, LauncherError> {
    let mut z = open(archive)?;
    let mut entry = z
        .by_name("modrinth.index.json")
        .map_err(|_| LauncherError::Parse {
            what: archive.display().to_string(),
            message: "not a Modrinth modpack (no modrinth.index.json)".into(),
        })?;
    let mut buf = Vec::new();
    (&mut entry)
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(LauncherError::io(archive))?;
    if buf.len() as u64 > MAX_INDEX_BYTES {
        return Err(LauncherError::Extract(ExtractError::TooLarge(
            MAX_INDEX_BYTES,
        )));
    }
    Ok(Index::parse(&buf)?.validate()?)
}

/// Download a modpack file from Modrinth's CDN into `cache_dir`, verified by
/// size, SHA-1 and SHA-512. Returns its path (named by SHA-1).
pub async fn fetch_pack(
    dl: &Downloader,
    cache_dir: &Path,
    file: &VersionFile,
    cancel: &CancellationToken,
) -> Result<PathBuf, LauncherError> {
    let sha1 = file.hashes.sha1.to_ascii_lowercase();
    if !trusted_file_url(&file.url)
        || sha1.len() != 40
        || !sha1.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(LauncherError::InvalidPath(file.url.clone()));
    }
    let dest = cache_dir.join(format!("{sha1}.mrpack"));
    dl.fetch_all(
        vec![DownloadTask {
            url: file.url.clone(),
            dest: dest.clone(),
            sha1: Some(sha1),
            size: (file.size > 0).then_some(file.size),
            root: None,
        }],
        cancel,
        |_| {},
    )
    .await?;
    let (d, want) = (dest.clone(), file.hashes.sha512.clone());
    let ok = tokio::task::spawn_blocking(move || {
        sha512_file(&d).is_ok_and(|got| got.eq_ignore_ascii_case(&want))
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))?;
    if !ok {
        let _ = std::fs::remove_file(&dest);
        return Err(LauncherError::Parse {
            what: file.filename.clone(),
            message: "SHA-512 does not match Modrinth".into(),
        });
    }
    Ok(dest)
}

/// Download `files` into `game_dir`, verified by size and SHA-1 (download
/// engine) and SHA-512 (here). A file that fails SHA-512 is deleted.
pub async fn download_files(
    dl: &Downloader,
    game_dir: &Path,
    files: &[PackFile],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Summary, LauncherError> {
    let root = Confined::new(game_dir).map_err(LauncherError::io(game_dir))?;
    for f in files {
        check_instance_path(&root, &f.path)?;
    }
    download_confined(dl, &root, files, cancel, on_progress).await
}

/// Refuse `rel` unless a pack may write it: [`instance_path_allowed`], and
/// its first folder on disk is not a reserved one reached through another
/// name (a Windows 8.3 short name such as `NC-UPD~1`).
pub(crate) fn check_instance_path(root: &Confined, rel: &str) -> Result<(), LauncherError> {
    if !instance_path_allowed(rel) {
        return Err(LauncherError::InvalidPath(rel.to_owned()));
    }
    let real = root
        .real_first(rel)
        .map_err(LauncherError::io(root.root().join(rel)))?;
    if real.is_some_and(|n| is_reserved_dir(&n.to_string_lossy())) {
        return Err(LauncherError::InvalidPath(rel.to_owned()));
    }
    Ok(())
}

/// [`download_files`] without the reserved-folder check, for the update's
/// own staging folder. Parent folders are created through `root`, and the
/// destination and its `.part` file must not be links.
pub(crate) async fn download_confined(
    dl: &Downloader,
    root: &Confined,
    files: &[PackFile],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Summary, LauncherError> {
    let (r, rels) = (
        root.clone(),
        files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
    );
    let dests = tokio::task::spawn_blocking(move || {
        rels.iter()
            .map(|rel| {
                let dest = r
                    .create_parent(rel)
                    .map_err(LauncherError::io(r.root().join(rel)))?;
                let part = format!("{rel}.part");
                r.path(&part)
                    .map_err(LauncherError::io(r.root().join(&part)))?;
                Ok(dest)
            })
            .collect::<Result<Vec<PathBuf>, LauncherError>>()
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))??;
    let mut tasks = Vec::with_capacity(files.len());
    let mut checks = Vec::with_capacity(files.len());
    for (f, dest) in files.iter().zip(dests) {
        tasks.push(DownloadTask {
            url: f.url.clone(),
            dest: dest.clone(),
            sha1: Some(f.sha1.clone()),
            size: Some(f.size),
            root: Some(root.clone()),
        });
        checks.push((dest, f.sha512.clone()));
    }
    let summary = dl.fetch_all(tasks, cancel, on_progress).await?;
    let r = root.clone();
    let bad = tokio::task::spawn_blocking(move || {
        checks
            .into_iter()
            .find_map(|(p, want)| match sha512_file(&p) {
                Ok(got) if got.eq_ignore_ascii_case(&want) => None,
                _ => {
                    r.remove_temp(&p);
                    Some(p)
                }
            })
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))?;
    if let Some(p) = bad {
        return Err(LauncherError::Parse {
            what: p.display().to_string(),
            message: "SHA-512 does not match the modpack".into(),
        });
    }
    Ok(summary)
}

/// A file the pack put into the instance, with its SHA-1 at that time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    /// Relative path in the game folder, `/`-separated.
    pub path: String,
    /// Lower-case hex SHA-1.
    pub sha1: String,
    pub is_override: bool,
}

/// Validate every override entry and pick, per target path
/// (case-insensitive), the one with the highest priority:
/// `client-overrides/` over `overrides/`. Returns entry indices in order.
fn chosen_overrides(
    z: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
) -> Result<Vec<usize>, LauncherError> {
    let mut chosen: HashMap<String, (usize, u8)> = HashMap::new();
    for i in 0..z.len() {
        let entry = z
            .by_index(i)
            .map_err(|e| LauncherError::Extract(e.into()))?;
        let Some(name) = entry.name().ok().map(|n| n.to_string()) else {
            return Err(LauncherError::Extract(ExtractError::UnsafeEntry(
                "<undecodable name>".into(),
            )));
        };
        let Some((rel, prio)) = override_target(&name) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() || !instance_path_allowed(rel) {
            return Err(LauncherError::Extract(ExtractError::UnsafeEntry(name)));
        }
        let key = rel.to_ascii_lowercase();
        match chosen.get(&key) {
            Some(&(_, p)) if p >= prio => {}
            _ => {
                chosen.insert(key, (i, prio));
            }
        }
    }
    let mut order: Vec<usize> = chosen.into_values().map(|(i, _)| i).collect();
    order.sort_unstable();
    Ok(order)
}

struct HashWriter<W> {
    inner: W,
    hash: sha1::Sha1,
}

impl<W: std::io::Write> std::io::Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        sha1::Digest::update(&mut self.hash, &buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Copy every chosen override to `dest(rel)`, a path relative to `root`
/// (a temporary file, then renamed), hashing as it goes. Returns what was
/// written and where (relative to `root`).
pub(crate) fn copy_overrides(
    archive: &Path,
    max_total: u64,
    root: &Confined,
    mut dest: impl FnMut(&str) -> Result<String, LauncherError>,
) -> Result<Vec<(Tracked, String)>, LauncherError> {
    let mut z = open(archive)?;
    let order = chosen_overrides(&mut z)?;
    let mut written = 0u64;
    let mut out = Vec::with_capacity(order.len());
    for i in order {
        let mut entry = z
            .by_index(i)
            .map_err(|e| LauncherError::Extract(e.into()))?;
        let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
        let Some((rel, _)) = override_target(&name) else {
            continue;
        };
        let target = dest(rel)?;
        let (file, part) = root
            .temp_for(&target)
            .map_err(LauncherError::io(root.root().join(&target)))?;
        let mut w = HashWriter {
            inner: file,
            hash: <sha1::Sha1 as sha1::Digest>::new(),
        };
        let remaining = max_total.saturating_sub(written);
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut w);
        let sha1 = hex::encode(sha1::Digest::finalize(w.hash));
        drop(w.inner);
        match copied {
            Ok(n) => written += n,
            Err(e) => {
                root.remove_temp(&part);
                return Err(LauncherError::io(&part)(e));
            }
        }
        if written > max_total {
            root.remove_temp(&part);
            return Err(LauncherError::Extract(ExtractError::TooLarge(max_total)));
        }
        root.persist(&part, &target)
            .map_err(LauncherError::io(root.root().join(&target)))?;
        out.push((
            Tracked {
                path: rel.to_owned(),
                sha1,
                is_override: true,
            },
            target,
        ));
    }
    Ok(out)
}

/// Copy the pack's overrides into `game_dir` (call off the async runtime).
/// `client-overrides/` wins over `overrides/` for the same path. Returns
/// the files written, with their SHA-1.
pub fn extract_overrides(
    archive: &Path,
    game_dir: &Path,
    max_total: u64,
) -> Result<Vec<Tracked>, LauncherError> {
    let root = Confined::new(game_dir).map_err(LauncherError::io(game_dir))?;
    Ok(copy_overrides(archive, max_total, &root, |rel| {
        check_instance_path(&root, rel)?;
        Ok(rel.to_owned())
    })?
    .into_iter()
    .map(|(t, _)| t)
    .collect())
}

/// What an import leaves in the instance: the pack's files, then its
/// overrides (an override replaces a downloaded file at the same path).
pub fn tracked_after_install(pack: &Pack, overrides: &[Tracked]) -> Vec<Tracked> {
    let mut by_path: HashMap<String, Tracked> = HashMap::new();
    for f in &pack.files {
        by_path.insert(
            f.path.to_ascii_lowercase(),
            Tracked {
                path: f.path.clone(),
                sha1: f.sha1.to_ascii_lowercase(),
                is_override: false,
            },
        );
    }
    for o in overrides {
        by_path.insert(o.path.to_ascii_lowercase(), o.clone());
    }
    let mut v: Vec<Tracked> = by_path.into_values().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn pack(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut w = zip::ZipWriter::new(f.reopen().unwrap());
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        f
    }

    const INDEX: &str = r#"{"formatVersion":1,"game":"minecraft","versionId":"1","name":"P",
        "files":[],"dependencies":{"minecraft":"1.21.1","fabric-loader":"0.16.10"}}"#;

    #[test]
    fn reads_and_validates_the_index() {
        let p = pack(&[("modrinth.index.json", INDEX.as_bytes())]);
        let r = read_pack(p.path()).unwrap();
        assert_eq!((r.name.as_str(), r.minecraft.as_str()), ("P", "1.21.1"));
        let none = pack(&[("other.json", b"{}")]);
        assert!(
            read_pack(none.path())
                .unwrap_err()
                .to_string()
                .contains("modrinth.index.json")
        );
        let bad = pack(&[("modrinth.index.json", br#"{"formatVersion":1}"#)]);
        assert!(read_pack(bad.path()).is_err());
    }

    #[test]
    fn overrides_client_wins_and_paths_are_checked() {
        let p = pack(&[
            ("modrinth.index.json", INDEX.as_bytes()),
            ("overrides/config/a.toml", b"common"),
            ("overrides/options.txt", b"common"),
            ("client-overrides/options.txt", b"client"),
            ("server-overrides/server.properties", b"x"),
        ]);
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            extract_overrides(p.path(), d.path(), 1 << 20)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            std::fs::read(d.path().join("options.txt")).unwrap(),
            b"client"
        );
        assert_eq!(
            std::fs::read(d.path().join("config/a.toml")).unwrap(),
            b"common"
        );
        assert!(!d.path().join("server.properties").exists());
        assert!(!d.path().join("modrinth.index.json").exists());

        for evil in [
            "overrides/../escape.txt",
            "overrides/.natives/x.dll",
            "client-overrides/C:/x",
        ] {
            let p = pack(&[(evil, b"x")]);
            let d = tempfile::tempdir().unwrap();
            assert!(
                extract_overrides(p.path(), &d.path().join("inst"), 1 << 20).is_err(),
                "{evil}"
            );
            assert!(!d.path().join("escape.txt").exists());
        }
        let big = pack(&[
            ("overrides/a.bin", &[0u8; 64]),
            ("overrides/b.bin", &[0u8; 64]),
        ]);
        let d = tempfile::tempdir().unwrap();
        assert!(extract_overrides(big.path(), d.path(), 100).is_err());
    }
}
