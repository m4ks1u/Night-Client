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
#[derive(Debug, Clone)]
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
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok((f, tmp))
    }

    /// Rename `tmp` onto `rel`, checking the destination again first. The
    /// temporary file is removed on failure.
    pub fn persist(&self, tmp: &Path, rel: &str) -> io::Result<PathBuf> {
        let done = (|| {
            let target = self.path(rel)?;
            if let Some(parent) = target.parent() {
                self.verify(parent)?;
            }
            fs::rename(tmp, &target)?;
            Ok(target)
        })();
        if done.is_err() {
            let _ = fs::remove_file(tmp);
        }
        done
    }

    /// Write `bytes` to `rel` atomically (temporary file, flushed, renamed).
    pub fn write(&self, rel: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let (mut f, tmp) = self.temp_for(rel)?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        self.persist(&tmp, rel)
    }

    /// Rename `from` to `to`, both inside the root and checked. `to`'s
    /// folder must exist.
    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let a = self.path(from)?;
        let b = self.path(to)?;
        for p in [a.parent(), b.parent()].into_iter().flatten() {
            self.verify(p)?;
        }
        fs::rename(&a, &b)
    }

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
        fs::write(outside.path().join(tmp.file_name().unwrap()), b"t").unwrap();
        assert!(c.persist(&tmp, "cfg/x.txt").is_err());
        assert!(!outside.path().join("x.txt").exists());
        fs::remove_dir(root.path().join("cfg")).unwrap();
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
