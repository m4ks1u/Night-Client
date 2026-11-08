//! Safe file-system helpers: confined path joins, atomic writes, hashing and
//! zip extraction that cannot escape its destination.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use nc_core::{CoreError, paths::validate_relative};
use sha1::{Digest, Sha1};

/// Join an untrusted `/`-separated relative path onto `root`.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, CoreError> {
    validate_relative(rel)?;
    let mut out = root.to_path_buf();
    for seg in rel.split('/') {
        out.push(seg);
    }
    debug_assert!(out.starts_with(root));
    Ok(out)
}

/// Write `bytes` to `path` atomically: write a sibling temp file, flush it to
/// disk, then rename over the destination.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent"))?;
    fs::create_dir_all(parent)?;
    // A random name, created exclusively: an existing file or link with a
    // guessable name is never opened.
    let tmp = crate::confined::temp_name(path)?;
    let written = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Lower-case hex SHA-1 of a file, streamed.
pub fn sha1_file(path: &Path) -> io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// True when `path` exists and matches the expected size and SHA-1.
pub fn is_valid(path: &Path, sha1: Option<&str>, size: Option<u64>) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || size.is_some_and(|s| s != meta.len()) {
        return false;
    }
    match sha1 {
        Some(expected) => sha1_file(path).is_ok_and(|h| h.eq_ignore_ascii_case(expected)),
        None => true,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("archive I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("archive entry `{0}` would escape the destination")]
    UnsafeEntry(String),
    #[error("archive exceeds the {0} byte extraction limit")]
    TooLarge(u64),
}

/// Extract a zip/jar into `dest`, skipping entries whose path starts with
/// one of `exclude` (e.g. `META-INF/`).
///
/// Every entry name is validated (no absolute paths, `..`, drive letters,
/// alternate data streams); a single unsafe entry aborts extraction.
/// Total uncompressed size is capped to defend against zip bombs. Writes
/// go through [`Confined`](crate::confined::Confined): a junction or
/// symlink already inside `dest` is never followed.
pub fn extract_zip(
    archive: &Path,
    dest: &Path,
    exclude: &[String],
    max_total: u64,
) -> Result<usize, ExtractError> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))?;
    let root = crate::confined::Confined::new(dest)?;
    let mut written = 0u64;
    let mut count = 0usize;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry
            .name()
            .map_err(|_| ExtractError::UnsafeEntry("<undecodable name>".into()))?
            .to_string();
        if exclude.iter().any(|e| name.starts_with(e.as_str())) {
            continue;
        }
        let rel = name.trim_end_matches('/');
        if rel.is_empty() {
            continue;
        }
        safe_join(dest, rel).map_err(|_| ExtractError::UnsafeEntry(name.clone()))?;
        if entry.is_dir() {
            root.create_dirs(rel)?;
            continue;
        }
        if entry.is_symlink() {
            return Err(ExtractError::UnsafeEntry(name));
        }
        let (mut out, tmp) = root.temp_for(rel)?;
        let remaining = max_total.saturating_sub(written);
        let copied = io::copy(&mut (&mut entry).take(remaining + 1), &mut out);
        drop(out);
        let n = match copied {
            Ok(n) => n,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e.into());
            }
        };
        written += n;
        if written > max_total {
            let _ = fs::remove_file(&tmp);
            return Err(ExtractError::TooLarge(max_total));
        }
        root.persist(&tmp, rel)?;
        count += 1;
    }
    Ok(count)
}

/// Delete leftover `*.part` (interrupted downloads) and `*.tmp` (interrupted
/// atomic writes) files older than `max_age` under `root`. Fresh ones are
/// kept so a download that is resumed soon is not thrown away. Never
/// follows symlinks/junctions. Returns (files removed, bytes freed).
pub fn sweep_stale(root: &Path, max_age: std::time::Duration) -> (u64, u64) {
    let now = std::time::SystemTime::now();
    let (mut n, mut bytes) = (0u64, 0u64);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let path = e.path();
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".part") || name.ends_with(".tmp")) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let old = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > max_age);
            if old && fs::remove_file(&path).is_ok() {
                n += 1;
                bytes += meta.len();
            }
        }
    }
    (n, bytes)
}

/// Free bytes available to the current user on the volume holding `path`.
/// The path (or its nearest existing ancestor) is used.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn free_space(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            avail: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }

    let mut p = path;
    while !p.exists() {
        p = p
            .parent()
            .ok_or_else(|| io::Error::other("no existing ancestor"))?;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut avail = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call;
    // the out-pointers point to valid u64s or are null (allowed by the API).
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(avail)
}

#[cfg(not(windows))]
pub fn free_space(_path: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_confines_paths() {
        let root = Path::new(r"C:\data");
        assert_eq!(
            safe_join(root, "a/b.jar").unwrap(),
            root.join("a").join("b.jar")
        );
        for bad in ["../x", "a/../../x", "C:/x", "/x", "a\\b", "x:ads"] {
            assert!(safe_join(root, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn atomic_write_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/file.txt");
        atomic_write(&p, b"abc").unwrap();
        assert_eq!(
            sha1_file(&p).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert!(is_valid(
            &p,
            Some("A9993E364706816ABA3E25717850C26C9CD0D89D"),
            Some(3)
        ));
        assert!(!is_valid(
            &p,
            Some("a9993e364706816aba3e25717850c26c9cd0d89d"),
            Some(4)
        ));
        assert!(!is_valid(
            &p,
            Some("0000000000000000000000000000000000000000"),
            None
        ));
        // No temp files left behind.
        assert_eq!(fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
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

    #[test]
    fn extract_respects_excludes() {
        let z = make_zip(&[
            ("lwjgl.dll", b"dll"),
            ("META-INF/MANIFEST.MF", b"m"),
            ("sub/x.dll", b"x"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let n = extract_zip(z.path(), dir.path(), &["META-INF/".into()], 1 << 20).unwrap();
        assert_eq!(n, 2);
        assert!(dir.path().join("lwjgl.dll").exists());
        assert!(dir.path().join("sub").join("x.dll").exists());
        assert!(!dir.path().join("META-INF").exists());
    }

    #[test]
    fn extract_rejects_zip_slip() {
        for evil in [
            "../evil.dll",
            "a/../../evil.dll",
            "C:/Windows/evil.dll",
            "x.dll:stream",
        ] {
            let z = make_zip(&[("ok.dll", b"ok"), (evil, b"pwn")]);
            let parent = tempfile::tempdir().unwrap();
            let dest = parent.path().join("natives");
            let err = extract_zip(z.path(), &dest, &[], 1 << 20).unwrap_err();
            assert!(matches!(err, ExtractError::UnsafeEntry(_)), "{evil}: {err}");
            assert!(!parent.path().join("evil.dll").exists());
        }
    }

    #[test]
    fn extract_enforces_size_cap() {
        let big = vec![0u8; 10_000];
        let z = make_zip(&[("big.bin", &big)]);
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract_zip(z.path(), dir.path(), &[], 1000),
            Err(ExtractError::TooLarge(_))
        ));
        assert!(!dir.path().join("big.bin").exists());
    }

    #[test]
    fn sweep_removes_only_old_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("libraries/a");
        fs::create_dir_all(&sub).unwrap();
        for f in ["x.jar.part", "y.json.123.tmp", "keep.jar", "fresh.part"] {
            fs::write(sub.join(f), b"data").unwrap();
        }
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
        for f in ["x.jar.part", "y.json.123.tmp", "keep.jar"] {
            fs::File::options()
                .write(true)
                .open(sub.join(f))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let (n, bytes) = sweep_stale(dir.path(), std::time::Duration::from_secs(86400));
        assert_eq!((n, bytes), (2, 8));
        assert!(sub.join("keep.jar").exists());
        assert!(sub.join("fresh.part").exists());
    }

    #[test]
    fn free_space_reports_something() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(&dir.path().join("does/not/exist")).unwrap() > 0);
    }
}
