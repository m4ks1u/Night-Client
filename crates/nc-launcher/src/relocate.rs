//! Moving the game-data folders (versions, libraries, assets, runtimes,
//! instances, cache, trash) to another location.
//!
//! The database stays in `%APPDATA%\NightClient`, and logs and the token vault
//! stay in `%LOCALAPPDATA%`. The chosen location is stored in
//! `<default root>/data-location.json` and read once at startup.
//!
//! A move is all-or-nothing: each folder is renamed (same drive) or copied
//! (another drive). If anything fails, renamed folders are renamed back and
//! copies are deleted. The new location is saved before any original is
//! deleted, so a crash at any point leaves one complete copy that the saved
//! location points to.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{LauncherError, fsutil};

/// Folders under the data root that hold game data and move together.
pub const MANAGED: [&str; 7] = [
    "versions",
    "libraries",
    "assets",
    "runtimes",
    "instances",
    "cache",
    "trash",
];

const LOCATION_FILE: &str = "data-location.json";

#[derive(Serialize, Deserialize)]
struct Location {
    root: PathBuf,
}

/// The data root to use: the saved location, or `default_root` when none is
/// saved or the saved one is not an absolute path.
pub fn resolve_root(default_root: &Path) -> PathBuf {
    let file = default_root.join(LOCATION_FILE);
    let Ok(text) = fs::read_to_string(&file) else {
        return default_root.to_path_buf();
    };
    match serde_json::from_str::<Location>(&text) {
        Ok(l) if l.root.is_absolute() => l.root,
        _ => {
            tracing::warn!(file = %file.display(), "ignoring an unreadable data location");
            default_root.to_path_buf()
        }
    }
}

/// Save `root` as the data location (removing the file when it is the default).
pub fn save_location(default_root: &Path, root: &Path) -> io::Result<()> {
    let file = default_root.join(LOCATION_FILE);
    if root == default_root {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        };
    }
    fs::create_dir_all(default_root)?;
    let json = serde_json::to_vec_pretty(&Location {
        root: root.to_path_buf(),
    })
    .map_err(io::Error::other)?;
    fsutil::atomic_write(&file, &json)
}

/// Check a user-chosen destination. It must be a local absolute path (game
/// data includes Java runtimes, which are executed, so no network shares),
/// not a drive root, not a Windows folder, not inside or around the current
/// data folder, and either missing or empty. The default root is always
/// allowed, so the data can be moved back. Returns the path to use.
pub fn validate_target(
    current: &Path,
    default_root: &Path,
    input: &str,
) -> Result<PathBuf, LauncherError> {
    let invalid = |m: &str| LauncherError::InvalidPath(format!("{input}: {m}"));
    let trimmed = input.trim();
    if trimmed.starts_with(r"\\") || trimmed.starts_with("//") {
        return Err(invalid("choose a folder on a local drive"));
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err(invalid("choose a full path, for example D:\\Night Client"));
    }
    if p.parent().is_none() {
        return Err(invalid("a drive root cannot be the data folder"));
    }
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid("the path must not contain . or .. segments"));
    }
    let tail = p.to_string_lossy();
    if tail.get(2..).unwrap_or("").contains(':')
        || tail
            .chars()
            .any(|c| c.is_control() || matches!(c, '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(invalid(
            "the path contains characters Windows does not allow",
        ));
    }
    let canon = canonical_lenient;
    let (target, cur) = (canon(&p), canon(current));
    if target == cur {
        return Err(invalid("the data is already in this folder"));
    }
    if target == canon(default_root) {
        return Ok(default_root.to_path_buf());
    }
    if target.starts_with(&cur) || cur.starts_with(&target) {
        return Err(invalid(
            "choose a folder that is neither inside nor around the current data folder",
        ));
    }
    let mut forbidden = crate::instance::system_dirs();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        forbidden.push(PathBuf::from(local).join("NightClient"));
    }
    if forbidden.iter().any(|f| target.starts_with(canon(f))) {
        return Err(invalid("this folder is managed by Windows or Night Client"));
    }
    match fs::read_dir(&p) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(invalid("choose an empty folder or a new one"));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(invalid(&e.to_string())),
    }
    Ok(p)
}

/// Canonical form of a path that may not exist yet: canonicalize the nearest
/// existing ancestor and re-append the rest, so paths compare in the same
/// (`\\?\`-prefixed) form.
fn canonical_lenient(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(c) = fs::canonicalize(cur) {
            return rest.iter().rev().fold(c, |acc, s| acc.join(s));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Total size of the managed folders under `root` (symlinks are not followed).
pub fn managed_size(root: &Path) -> u64 {
    MANAGED.iter().map(|d| tree_size(&root.join(d))).sum()
}

fn tree_size(p: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(p) else {
        return 0;
    };
    if meta.is_dir() {
        fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
            .unwrap_or(0)
    } else if meta.is_file() {
        meta.len()
    } else {
        0
    }
}

enum Moved {
    Renamed(PathBuf, PathBuf),
    Copied(PathBuf, PathBuf),
}

/// Move the managed folders from `from` to `to`, call `commit` (which saves
/// the new location), then delete the originals of copied folders.
/// `progress(done, total)` reports copied bytes. With `force_copy`, folders
/// are copied even when a rename would work (used by tests).
pub fn relocate(
    from: &Path,
    to: &Path,
    force_copy: bool,
    progress: &dyn Fn(u64, u64),
    commit: impl FnOnce() -> io::Result<()>,
) -> Result<(), LauncherError> {
    fs::create_dir_all(to).map_err(LauncherError::io(to))?;
    let total = managed_size(from);
    let mut done = 0u64;
    let mut moved: Vec<Moved> = Vec::new();

    let result = (|| -> Result<(), LauncherError> {
        for d in MANAGED {
            let (src, dst) = (from.join(d), to.join(d));
            if fs::symlink_metadata(&src).is_err() {
                continue;
            }
            if dst.exists() {
                return Err(LauncherError::InvalidPath(format!(
                    "{}: already exists",
                    dst.display()
                )));
            }
            if !force_copy && fs::rename(&src, &dst).is_ok() {
                done += tree_size(&dst);
                progress(done, total);
                moved.push(Moved::Renamed(src, dst));
                continue;
            }
            // Record before copying so a partial copy is cleaned up too.
            moved.push(Moved::Copied(src.clone(), dst.clone()));
            copy_tree(&src, &dst, &mut done, total, progress)?;
        }
        commit().map_err(LauncherError::io(to))
    })();

    if let Err(e) = result {
        for m in moved.into_iter().rev() {
            match m {
                Moved::Renamed(src, dst) => {
                    if let Err(re) = fs::rename(&dst, &src) {
                        tracing::error!(from = %dst.display(), to = %src.display(), error = %re, "could not move a folder back");
                    }
                }
                Moved::Copied(_, dst) => {
                    let _ = fs::remove_dir_all(&dst);
                }
            }
        }
        return Err(e);
    }

    // The new location is saved; the copied originals are now redundant.
    for m in moved {
        if let Moved::Copied(src, _) = m
            && let Err(e) = fs::remove_dir_all(&src)
        {
            tracing::warn!(path = %src.display(), error = %e, "could not delete the old copy");
        }
    }
    Ok(())
}

/// Copy a directory tree, verifying each file's size. Symlinks and
/// junctions are refused rather than followed.
fn copy_tree(
    src: &Path,
    dst: &Path,
    done: &mut u64,
    total: u64,
    progress: &dyn Fn(u64, u64),
) -> Result<(), LauncherError> {
    let meta = fs::symlink_metadata(src).map_err(LauncherError::io(src))?;
    if meta.is_symlink() {
        return Err(LauncherError::InvalidPath(format!(
            "{}: links are not moved; remove it and try again",
            src.display()
        )));
    }
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(LauncherError::io(dst))?;
        for entry in fs::read_dir(src).map_err(LauncherError::io(src))? {
            let entry = entry.map_err(LauncherError::io(src))?;
            copy_tree(
                &entry.path(),
                &dst.join(entry.file_name()),
                done,
                total,
                progress,
            )?;
        }
    } else {
        let n = fs::copy(src, dst).map_err(LauncherError::io(dst))?;
        if n != meta.len() {
            return Err(LauncherError::Io {
                path: dst.to_path_buf(),
                source: io::Error::other("copied size does not match"),
            });
        }
        *done += n;
        progress(*done, total);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn seed(root: &Path) {
        for (p, body) in [
            ("versions/1.21.1/1.21.1.json", "{}"),
            ("libraries/a/b.jar", "jar-bytes"),
            ("instances/World/saves/level.dat", "level"),
            ("cache/manifest.json", "[]"),
        ] {
            let f = root.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        }
        fs::write(root.join("night-client.db"), "db").unwrap();
    }

    fn check_moved(from: &Path, to: &Path) {
        assert_eq!(
            fs::read_to_string(to.join("libraries/a/b.jar")).unwrap(),
            "jar-bytes"
        );
        assert_eq!(
            fs::read_to_string(to.join("instances/World/saves/level.dat")).unwrap(),
            "level"
        );
        for d in MANAGED {
            assert!(!from.join(d).exists(), "{d} left behind");
        }
        // The database is not game data and stays put.
        assert!(from.join("night-client.db").is_file());
        assert!(!to.join("night-client.db").exists());
    }

    #[test]
    fn renames_on_the_same_drive_and_copies_otherwise() {
        for force_copy in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let (from, to) = (t.path().join("old"), t.path().join("new"));
            seed(&from);
            let total = managed_size(&from);
            let last = Cell::new(0);
            relocate(
                &from,
                &to,
                force_copy,
                &|d, n| {
                    assert_eq!(n, total);
                    last.set(d);
                },
                || Ok(()),
            )
            .unwrap();
            assert_eq!(last.get(), total, "progress reaches the total");
            check_moved(&from, &to);
        }
    }

    #[test]
    fn a_failed_commit_puts_everything_back() {
        for force_copy in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let (from, to) = (t.path().join("old"), t.path().join("new"));
            seed(&from);
            let err = relocate(&from, &to, force_copy, &|_, _| {}, || {
                Err(io::Error::other("disk full"))
            });
            assert!(err.is_err());
            assert_eq!(
                fs::read_to_string(from.join("libraries/a/b.jar")).unwrap(),
                "jar-bytes"
            );
            assert!(from.join("versions/1.21.1/1.21.1.json").is_file());
            for d in MANAGED {
                assert!(
                    !to.join(d).exists(),
                    "{d} not cleaned up (force_copy={force_copy})"
                );
            }
        }
    }

    #[test]
    fn location_round_trips_and_defaults() {
        let t = tempfile::tempdir().unwrap();
        let def = t.path().join("NightClient");
        assert_eq!(resolve_root(&def), def);
        let other = t.path().join("D-drive").join("Night Client");
        save_location(&def, &other).unwrap();
        assert_eq!(resolve_root(&def), other);
        save_location(&def, &def).unwrap();
        assert_eq!(resolve_root(&def), def);
        fs::write(def.join(LOCATION_FILE), r#"{"root":"relative\\path"}"#).unwrap();
        assert_eq!(resolve_root(&def), def, "relative paths are ignored");
        fs::write(def.join(LOCATION_FILE), "garbage").unwrap();
        assert_eq!(resolve_root(&def), def);
    }

    #[test]
    fn targets_are_validated() {
        let t = tempfile::tempdir().unwrap();
        let def = t.path().join("NightClient");
        let cur = t.path().join("Current");
        fs::create_dir_all(&cur).unwrap();
        let ok = t.path().join("Fresh");
        assert_eq!(
            validate_target(&cur, &def, ok.to_str().unwrap()).unwrap(),
            ok
        );
        // Moving back to the default is always allowed.
        assert_eq!(
            validate_target(&cur, &def, def.to_str().unwrap()).unwrap(),
            def
        );

        let full = t.path().join("Full");
        fs::create_dir_all(&full).unwrap();
        fs::write(full.join("x"), "x").unwrap();
        let bad = [
            r"\\server\share\nc".to_owned(),
            "relative\\dir".to_owned(),
            "C:\\".to_owned(),
            format!("{}\\..\\x", ok.display()),
            format!("{}\\a:stream", ok.display()),
            cur.display().to_string(),
            cur.join("inside").display().to_string(),
            t.path().display().to_string(),
            full.display().to_string(),
        ];
        for b in bad {
            assert!(validate_target(&cur, &def, &b).is_err(), "accepted {b}");
        }
        if let Some(win) = std::env::var_os("windir") {
            let w = PathBuf::from(win).join("NightClientTest");
            assert!(validate_target(&cur, &def, w.to_str().unwrap()).is_err());
        }
    }
}
