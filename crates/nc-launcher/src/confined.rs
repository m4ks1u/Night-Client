//! Writes confined to a root folder.
//!
//! [`crate::fsutil::safe_join`] checks a path as text: no `..`, no absolute
//! paths. That is not enough on disk: a directory inside the root may be a
//! junction or symlink that points elsewhere, and writing "inside" it then
//! writes outside the root. Every write that takes a path from untrusted
//! data (archive entries, modpack indexes) goes through [`Confined`]:
//!
//! - Each existing component under the root is checked with
//!   `symlink_metadata`. Symlinks and junctions (Windows name-surrogate
//!   reparse points, which `FileType::is_symlink` reports) are refused.
//!   Other reparse points, like OneDrive placeholders, are ordinary files.
//! - Directories are created one level at a time and checked again.
//! - Temporary files get a random name and are created exclusively, so a
//!   pre-planted file or link with the expected name is never opened.
//! - Before a rename, the destination's folder is resolved and must still
//!   be inside the root, which catches a folder swapped after the check.
//!
//! The root itself is trusted: the user (or the launcher) chose it, and it
//! may be a junction, e.g. an instance folder moved to another drive.
//!
//! Residual risk: a process running as the same user that swaps a folder
//! for a junction between the final check and the rename can still win the
//! race. Closing that needs handle-relative Windows APIs; a process with the
//! user's rights can already write anywhere the user can.

use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use nc_core::paths::validate_relative;

/// A root folder that writes are confined to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confined {
    root: PathBuf,
    /// The root with links resolved, for [`Confined::verify`].
    real: PathBuf,
}

/// The error for a path that goes through a link.
pub fn link_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{} is a link (symlink or junction); refusing to write through it",
            path.display()
        ),
    )
}

fn not_dir(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} is a file where a folder is needed", path.display()),
    )
}

fn outside(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{} resolves outside its folder", path.display()),
    )
}

/// `symlink_metadata`, with "not found" as `None`.
pub fn lstat(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// A random hex string from the OS RNG.
pub fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| io::Error::other(format!("OS RNG unavailable: {e}")))?;
    Ok(hex::encode(buf))
}

/// A fresh, unpredictable sibling name for a temporary file next to `target`.
/// It ends in `.tmp`, so an interrupted write is swept as stale later.
pub fn temp_name(target: &Path) -> io::Result<PathBuf> {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", random_hex(8)?));
    Ok(target.with_file_name(name))
}

impl Confined {
    /// Confine writes to `root`, creating it if needed.
    pub fn new(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let real = fs::canonicalize(root)?;
        Ok(Self {
            root: root.to_owned(),
            real,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path of `rel` (`/`-separated). Every component that exists must
    /// not be a link, and every one but the last must be a folder. The
    /// path may not exist yet.
    pub fn path(&self, rel: &str) -> io::Result<PathBuf> {
        validate_relative(rel).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let segs: Vec<&str> = rel.split('/').collect();
        let mut p = self.root.clone();
        let mut exists = true;
        for (i, seg) in segs.iter().enumerate() {
            p.push(seg);
            if !exists {
                continue;
            }
            match lstat(&p)? {
                None => exists = false,
                Some(m) if m.file_type().is_symlink() => return Err(link_error(&p)),
                Some(m) if i + 1 < segs.len() && !m.is_dir() => return Err(not_dir(&p)),
                Some(_) => {}
            }
        }
        Ok(p)
    }

    /// Create the folder `rel` and its parents one level at a time; none may
    /// be a link or a file.
    pub fn create_dirs(&self, rel: &str) -> io::Result<PathBuf> {
        validate_relative(rel).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut p = self.root.clone();
        for seg in rel.split('/') {
            p.push(seg);
            match fs::create_dir(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    // A file in the way reports NotFound or similar on some
                    // systems; report the clearer reason when there is one.
                    return match lstat(&p) {
                        Ok(Some(m)) if m.file_type().is_symlink() => Err(link_error(&p)),
                        Ok(Some(m)) if !m.is_dir() => Err(not_dir(&p)),
                        _ => Err(e),
                    };
                }
            }
            match lstat(&p)? {
                Some(m) if m.file_type().is_symlink() => return Err(link_error(&p)),
                Some(m) if m.is_dir() => {}
                _ => return Err(not_dir(&p)),
            }
        }
        Ok(p)
    }

    /// The checked path of `rel`, with its parent folders created.
    pub fn create_parent(&self, rel: &str) -> io::Result<PathBuf> {
        if let Some((dir, _)) = rel.rsplit_once('/') {
            self.create_dirs(dir)?;
        }
        self.path(rel)
    }

    /// Fail unless the existing `path` resolves inside the root.
    pub fn verify(&self, path: &Path) -> io::Result<()> {
        let real = fs::canonicalize(path)?;
        if real.starts_with(&self.real) {
            Ok(())
        } else {
            Err(outside(path))
        }
    }

    /// A new, exclusively created temporary file next to `rel` (parents
    /// created). Move it into place with [`Confined::persist`].
    pub fn temp_for(&self, rel: &str) -> io::Result<(fs::File, PathBuf)> {
        let target = self.create_parent(rel)?;
        let tmp = temp_name(&target)?;
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        if let Err(e) = self.verify(&tmp) {
            drop(f);
            self.remove_temp(&tmp);
            return Err(e);
        }
        Ok((f, tmp))
    }

    /// Rename `tmp` onto `rel` (replacing it), checking the destination again
    /// first. The temporary file is removed on failure, when that is safe.
    pub fn persist(&self, tmp: &Path, rel: &str) -> io::Result<PathBuf> {
        let done = (|| {
            self.check(tmp)?;
            let target = self.path(rel)?;
            if let Some(parent) = target.parent() {
                self.verify(parent)?;
            }
            fs::rename(tmp, &target)?;
            Ok(target)
        })();
        if done.is_err() {
            self.remove_temp(tmp);
        }
        done
    }

    /// Write `bytes` to `rel` atomically (temporary file, flushed, renamed).
    pub fn write(&self, rel: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let (mut f, tmp) = self.temp_for(rel)?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written {
            self.remove_temp(&tmp);
            return Err(e);
        }
        self.persist(&tmp, rel)
    }

    /// Rename `from` to `to`, both inside the root and checked. `to`'s
    /// folder must exist. **Never replaces** an existing `to`: that fails
    /// atomically instead (no check-then-rename window).
    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let a = self.path(from)?;
        let b = self.path(to)?;
        for p in [a.parent(), b.parent()].into_iter().flatten() {
            self.verify(p)?;
        }
        rename_noreplace(&a, &b)
    }

    /// `abs` relative to the root, `/`-separated. Fails for a path outside
    /// the root (as text) or with `..`/prefix components.
    pub fn rel_of(&self, abs: &Path) -> io::Result<String> {
        let rest = abs.strip_prefix(&self.root).map_err(|_| outside(abs))?;
        let mut segs = Vec::new();
        for c in rest.components() {
            match c {
                std::path::Component::Normal(s) => {
                    segs.push(s.to_str().ok_or_else(|| outside(abs))?.to_owned())
                }
                _ => return Err(outside(abs)),
            }
        }
        if segs.is_empty() {
            return Err(outside(abs));
        }
        Ok(segs.join("/"))
    }

    /// Check an absolute path below the root as [`Confined::path`] does, and
    /// that its folder, when it exists, still resolves inside the root.
    pub fn check(&self, abs: &Path) -> io::Result<()> {
        let p = self.path(&self.rel_of(abs)?)?;
        if let Some(parent) = p.parent()
            && lstat(parent)?.is_some()
        {
            self.verify(parent)?;
        }
        Ok(())
    }

    /// Remove a temporary file this code created, **only** when its path is
    /// still trusted: no link on the way and the folder still inside the
    /// root. Otherwise it is left where it is (and logged): after a folder
    /// was swapped for a junction, the same path names someone else's file.
    /// Returns whether it was removed.
    pub fn remove_temp(&self, abs: &Path) -> bool {
        match self.check(abs) {
            Ok(()) => fs::remove_file(abs).is_ok(),
            Err(e) => {
                tracing::warn!(path = %abs.display(), error = %e, "left a temporary file in place: its folder is no longer trusted");
                false
            }
        }
    }
}

/// Rename without ever replacing an existing destination, atomically.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let wide = |p: &Path| -> Vec<u16> {
        verbatim(p)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect()
    };
    let (a, b) = (wide(from), wide(to));
    // SAFETY: both are NUL-terminated UTF-16 strings that outlive the call.
    // Flags 0: no MOVEFILE_REPLACE_EXISTING (fails if `to` exists) and no
    // MOVEFILE_COPY_ALLOWED (same volume only, so it stays a rename).
    let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// An absolute path in `\\?\` form, so long paths work with the raw API.
#[cfg(windows)]
fn verbatim(p: &Path) -> PathBuf {
    let s = p.as_os_str().to_string_lossy();
    let plain = p.is_absolute()
        && !s.starts_with(r"\\")
        && !p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        });
    if plain {
        PathBuf::from(format!(r"\\?\{}", s.replace('/', r"\")))
    } else {
        p.to_owned()
    }
}

/// Rename without ever replacing an existing destination: a hard link fails
/// when `to` exists; then the old name goes.
#[cfg(not(windows))]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    fs::hard_link(from, to)?;
    fs::remove_file(from)
}

impl Confined {
    /// The real on-disk name of `rel`'s first component, when it exists. On
    /// Windows this resolves 8.3 short names (`NC-UPD~1`) to the long name,
    /// so reserved folders can't be reached through an alias.
    pub fn real_first(&self, rel: &str) -> io::Result<Option<OsString>> {
        let first = rel.split('/').next().unwrap_or(rel);
        let p = self.root.join(first);
        if lstat(&p)?.is_none() {
            return Ok(None);
        }
        Ok(fs::canonicalize(&p)?.file_name().map(|n| n.to_os_string()))
    }
}

/// Create a directory junction at `link` pointing to `target`, for tests.
#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used)]
pub(crate) fn make_junction(link: &Path, target: &Path) {
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_checks_and_writes_inside_the_root() {
        let d = tempfile::tempdir().unwrap();
        let c = Confined::new(d.path()).unwrap();
        c.write("a/b/c.txt", b"x").unwrap();
        assert_eq!(fs::read(d.path().join("a/b/c.txt")).unwrap(), b"x");
        assert!(c.path("a/b/missing/deeper.txt").is_ok());
        // A file where a folder is needed.
        assert!(c.path("a/b/c.txt/d").is_err());
        assert!(c.create_dirs("a/b/c.txt/d").is_err());
        assert!(c.write("a/b/c.txt/d", b"x").is_err());
        for bad in ["../x", "a/../../x", "C:/x", "a\\b"] {
            assert!(c.path(bad).is_err(), "{bad}");
        }
        // No temporary files left behind.
        let left: Vec<_> = fs::read_dir(d.path().join("a/b")).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn temp_names_are_unpredictable() {
        let a = temp_name(Path::new("x/y.txt")).unwrap();
        let b = temp_name(Path::new("x/y.txt")).unwrap();
        assert_ne!(a, b);
        assert!(a.to_string_lossy().ends_with(".tmp"));
    }

    #[cfg(windows)]
    #[test]
    fn junctions_inside_the_root_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        make_junction(&root.path().join("linked"), outside.path());
        let c = Confined::new(root.path()).unwrap();
        assert!(c.path("linked/probe.txt").is_err());
        assert!(c.path("linked").is_err());
        assert!(c.create_dirs("linked/sub").is_err());
        assert!(c.write("linked/probe.txt", b"x").is_err());
        assert!(c.write("linked/sentinel", b"x").is_err());
        assert!(fs::read_dir(outside.path()).unwrap().count() == 1);
        assert_eq!(fs::read(outside.path().join("sentinel")).unwrap(), b"keep");
        fs::remove_dir(root.path().join("linked")).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_root_that_is_itself_a_junction_is_allowed() {
        let base = tempfile::tempdir().unwrap();
        let real = tempfile::tempdir().unwrap();
        let root = base.path().join("moved-instance");
        make_junction(&root, real.path());
        let c = Confined::new(&root).unwrap();
        c.write("mods/a.jar", b"x").unwrap();
        assert_eq!(fs::read(real.path().join("mods/a.jar")).unwrap(), b"x");
        fs::remove_dir(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_swapped_for_a_junction_after_the_check_is_caught() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let c = Confined::new(root.path()).unwrap();
        let (f, tmp) = c.temp_for("cfg/x.txt").unwrap();
        drop(f);
        // Replace `cfg` with a junction to outside: the temporary file moves
        // with the real folder, so put one at the same name outside too.
        let moved = root.path().join("cfg-real");
        fs::rename(root.path().join("cfg"), &moved).unwrap();
        make_junction(&root.path().join("cfg"), outside.path());
        let sentinel = outside.path().join(tmp.file_name().unwrap());
        fs::write(&sentinel, b"external sentinel").unwrap();
        assert!(c.persist(&tmp, "cfg/x.txt").is_err());
        assert!(!outside.path().join("x.txt").exists());
        // Re-review RR-002: the failed persist must not clean up through the
        // junction, deleting the outside file that now has the temp's name.
        assert_eq!(fs::read(&sentinel).unwrap(), b"external sentinel");
        assert!(!c.remove_temp(&tmp));
        assert_eq!(fs::read(&sentinel).unwrap(), b"external sentinel");
        // The real temporary file is still in the moved folder, untouched.
        assert!(moved.join(tmp.file_name().unwrap()).exists());
        fs::remove_dir(root.path().join("cfg")).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_write_never_cleans_up_through_a_swapped_folder() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let c = Confined::new(root.path()).unwrap();
        // Any write whose folder is a junction is refused before a temp file
        // exists, so nothing outside can be named by a cleanup.
        make_junction(&root.path().join("cfg"), outside.path());
        fs::write(outside.path().join("x.txt"), b"keep").unwrap();
        assert!(c.write("cfg/x.txt", b"x").is_err());
        assert!(c.temp_for("cfg/x.txt").is_err());
        assert!(!c.remove_temp(&root.path().join("cfg/x.txt")));
        assert_eq!(fs::read(outside.path().join("x.txt")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
        fs::remove_dir(root.path().join("cfg")).unwrap();
    }

    #[test]
    fn rename_never_replaces_and_paths_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let c = Confined::new(d.path()).unwrap();
        c.write("a.txt", b"a").unwrap();
        c.write("b.txt", b"b").unwrap();
        let e = c.rename("a.txt", "b.txt").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
        assert_eq!(fs::read(d.path().join("b.txt")).unwrap(), b"b");
        assert_eq!(fs::read(d.path().join("a.txt")).unwrap(), b"a");
        c.rename("a.txt", "c.txt").unwrap();
        assert!(!d.path().join("a.txt").exists());
        // A long path still works with the raw API.
        let deep = format!("{}/f.txt", ["segment-of-thirty-characters-x"; 9].join("/"));
        c.write(&deep, b"x").unwrap();
        c.rename(&deep, &format!("{deep}.moved")).unwrap();
        // Absolute paths: inside is fine, outside or `..` is refused.
        assert_eq!(c.rel_of(&d.path().join("x/y.txt")).unwrap(), "x/y.txt");
        assert!(c.rel_of(Path::new("C:/elsewhere/x")).is_err());
        assert!(c.rel_of(&d.path().join("x/../../y")).is_err());
        assert!(c.check(&d.path().join("c.txt")).is_ok());
        assert!(!c.remove_temp(Path::new("C:/elsewhere/x.tmp")));
        assert!(c.remove_temp(&d.path().join("c.txt")));
    }

    #[test]
    fn file_symlinks_are_refused_when_they_can_be_made() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim.txt");
        fs::write(&target, b"keep").unwrap();
        let link = root.path().join("victim.txt");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link).is_ok();
        if !made {
            eprintln!("skipped: creating file symlinks needs Developer Mode or admin rights");
            return;
        }
        let c = Confined::new(root.path()).unwrap();
        assert!(c.path("victim.txt").is_err());
        assert!(c.write("victim.txt", b"x").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }
}
