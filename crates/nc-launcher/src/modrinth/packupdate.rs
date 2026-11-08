//! Updating an instance made from a modpack to another version of the pack,
//! keeping what the user changed.
//!
//! Every file the pack put into the instance is tracked with its SHA-1 at
//! install time. For each path, the update compares three hashes: the one
//! recorded at install time, the one on disk now, and the one in the new pack.
//! - A file that is still as the pack left it is replaced or removed.
//! - A file the user changed, deleted, or that the pack didn't put there is
//!   kept, and listed.
//! - A file the user disabled (`<name>.disabled`) stays disabled when replaced.
//!
//! Everything new is first downloaded (verified) or extracted into a staging
//! folder inside the game folder; only then are files moved into place. A
//! failed download changes nothing.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use nc_core::mrpack::{Pack, PackFile};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::modpack::{self, Tracked};
use crate::{
    LauncherError,
    download::{Downloader, Progress},
    fsutil,
};

/// Staging folder inside the game folder (same volume, so moves are renames).
pub const STAGING: &str = ".nc-update";

/// A path's state on disk now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnDisk {
    Missing,
    Present(String),
    /// Only `<path>.disabled` exists (a mod or pack the user turned off).
    Disabled(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Not there before; written.
    Add,
    /// As the pack left it; replaced by the new version.
    Replace,
    /// Disabled by the user and unchanged; the new version is written disabled.
    ReplaceDisabled,
    /// Already identical to the new version.
    Same,
    /// No longer in the pack and unchanged; deleted.
    Remove,
    /// Disabled, unchanged and no longer in the pack; deleted.
    RemoveDisabled,
    /// Changed, deleted or not put there by the pack: left as it is.
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub path: String,
    pub action: Action,
    /// For writes: the new file.
    pub new: Option<Tracked>,
}

/// Decide what to do with every path the old or new pack version touches.
/// `new` must hold one entry per path (as from `tracked_after_install`).
/// Returns the steps and what to track afterwards.
pub fn plan(
    old: &[Tracked],
    new: &[Tracked],
    on_disk: &dyn Fn(&str) -> OnDisk,
) -> (Vec<Step>, Vec<Tracked>) {
    let old_by: HashMap<String, &Tracked> = old
        .iter()
        .map(|t| (t.path.to_ascii_lowercase(), t))
        .collect();
    let mut steps = Vec::new();
    let mut track = Vec::new();
    let mut seen = HashSet::new();
    for n in new {
        let key = n.path.to_ascii_lowercase();
        seen.insert(key.clone());
        let was = old_by.get(&key).map(|t| t.sha1.as_str());
        let action = match (on_disk(&n.path), was) {
            (OnDisk::Missing, None) => Action::Add,
            // The user deleted a file the pack installed: respect that.
            (OnDisk::Missing, Some(_)) => Action::Keep,
            (OnDisk::Present(c), _) if c == n.sha1 => Action::Same,
            (OnDisk::Present(c), Some(o)) if c == o => Action::Replace,
            (OnDisk::Present(_), _) => Action::Keep,
            (OnDisk::Disabled(c), _) if c == n.sha1 => Action::Same,
            (OnDisk::Disabled(c), Some(o)) if c == o => Action::ReplaceDisabled,
            (OnDisk::Disabled(_), _) => Action::Keep,
        };
        match action {
            Action::Keep => {
                // Keep tracking the install-time hash, so the next update
                // still sees the file as the user's.
                if let Some(o) = old_by.get(&key) {
                    track.push((*o).clone());
                }
            }
            _ => track.push(n.clone()),
        }
        let writes = matches!(
            action,
            Action::Add | Action::Replace | Action::ReplaceDisabled
        );
        steps.push(Step {
            path: n.path.clone(),
            action,
            new: writes.then(|| n.clone()),
        });
    }
    for o in old {
        if seen.contains(&o.path.to_ascii_lowercase()) {
            continue;
        }
        let action = match on_disk(&o.path) {
            OnDisk::Missing => continue,
            OnDisk::Present(c) if c == o.sha1 => Action::Remove,
            OnDisk::Disabled(c) if c == o.sha1 => Action::RemoveDisabled,
            _ => Action::Keep,
        };
        steps.push(Step {
            path: o.path.clone(),
            action,
            new: None,
        });
    }
    (steps, track)
}

fn disabled(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".disabled");
    PathBuf::from(s)
}

/// Read a path's state (call off the async runtime).
pub fn scan(game_dir: &Path, rel: &str) -> OnDisk {
    let Ok(p) = fsutil::safe_join(game_dir, rel) else {
        return OnDisk::Missing;
    };
    if p.is_file() {
        return fsutil::sha1_file(&p).map_or(OnDisk::Missing, OnDisk::Present);
    }
    let d = disabled(&p);
    if d.is_file() {
        return fsutil::sha1_file(&d).map_or(OnDisk::Missing, OnDisk::Disabled);
    }
    OnDisk::Missing
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReport {
    pub added: usize,
    pub replaced: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Paths left as the user had them.
    pub kept: Vec<String>,
    #[serde(skip)]
    pub tracked: Vec<Tracked>,
}

/// Update `game_dir` from the pack in `archive` (already read and validated
/// as `pack`). `old` is what the previous version left.
pub async fn update(
    dl: &Downloader,
    game_dir: &Path,
    archive: &Path,
    pack: &Pack,
    old: &[Tracked],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<UpdateReport, LauncherError> {
    let staging = game_dir.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(LauncherError::io(&staging))?;
    let result = stage_and_apply(
        dl,
        game_dir,
        &staging,
        archive,
        pack,
        old,
        cancel,
        on_progress,
    )
    .await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[allow(clippy::too_many_arguments)]
async fn stage_and_apply(
    dl: &Downloader,
    game_dir: &Path,
    staging: &Path,
    archive: &Path,
    pack: &Pack,
    old: &[Tracked],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<UpdateReport, LauncherError> {
    // 1. Overrides into staging (hashed while copying), then the plan.
    let (arch, stage) = (archive.to_owned(), staging.join("o"));
    let staged_overrides = tokio::task::spawn_blocking(move || {
        let mut n = 0usize;
        modpack::copy_overrides(&arch, modpack::MAX_OVERRIDES_BYTES, |_| {
            n += 1;
            Ok(stage.join(n.to_string()))
        })
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))??;
    let overrides: Vec<Tracked> = staged_overrides.iter().map(|(t, _)| t.clone()).collect();
    let new = modpack::tracked_after_install(pack, &overrides);
    let (gd, old_v, new_v) = (game_dir.to_owned(), old.to_vec(), new.clone());
    let (steps, tracked) =
        tokio::task::spawn_blocking(move || plan(&old_v, &new_v, &|rel| scan(&gd, rel)))
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))?;

    // 2. Download the pack files that will be written, into staging.
    let staged_override: HashMap<String, PathBuf> = staged_overrides
        .into_iter()
        .map(|(t, p)| (t.path.to_ascii_lowercase(), p))
        .collect();
    let files_by: HashMap<String, &PackFile> = pack
        .files
        .iter()
        .map(|f| (f.path.to_ascii_lowercase(), f))
        .collect();
    let mut downloads = Vec::new();
    let mut sources: HashMap<String, PathBuf> = HashMap::new();
    for s in &steps {
        let Some(n) = &s.new else { continue };
        let key = n.path.to_ascii_lowercase();
        if n.is_override {
            sources.insert(key.clone(), staged_override[&key].clone());
        } else {
            let f = files_by[&key];
            let rel = format!("{STAGING}/d/{}", downloads.len());
            sources.insert(
                key,
                game_dir
                    .join(STAGING)
                    .join("d")
                    .join(downloads.len().to_string()),
            );
            downloads.push(PackFile {
                path: rel,
                ..(*f).clone()
            });
        }
    }
    modpack::download_files(dl, game_dir, &downloads, cancel, on_progress).await?;
    if cancel.is_cancelled() {
        return Err(LauncherError::Cancelled);
    }

    // 3. Apply: everything is staged and verified.
    let gd = game_dir.to_owned();
    tokio::task::spawn_blocking(move || apply(&gd, &steps, &sources, tracked))
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))?
}

fn apply(
    game_dir: &Path,
    steps: &[Step],
    sources: &HashMap<String, PathBuf>,
    tracked: Vec<Tracked>,
) -> Result<UpdateReport, LauncherError> {
    let mut r = UpdateReport {
        tracked,
        ..Default::default()
    };
    for s in steps {
        let target = fsutil::safe_join(game_dir, &s.path)?;
        match s.action {
            Action::Add | Action::Replace | Action::ReplaceDisabled => {
                let src = &sources[&s.path.to_ascii_lowercase()];
                let dest = if s.action == Action::ReplaceDisabled {
                    disabled(&target)
                } else {
                    target
                };
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p).map_err(LauncherError::io(p))?;
                }
                std::fs::rename(src, &dest).map_err(LauncherError::io(&dest))?;
                if s.action == Action::Add {
                    r.added += 1;
                } else {
                    r.replaced += 1;
                }
            }
            Action::Remove => {
                std::fs::remove_file(&target).map_err(LauncherError::io(&target))?;
                r.removed += 1;
            }
            Action::RemoveDisabled => {
                let d = disabled(&target);
                std::fs::remove_file(&d).map_err(LauncherError::io(&d))?;
                r.removed += 1;
            }
            Action::Same => r.unchanged += 1,
            Action::Keep => r.kept.push(s.path.clone()),
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, sha1: &str) -> Tracked {
        Tracked {
            path: path.into(),
            sha1: sha1.into(),
            is_override: false,
        }
    }

    #[test]
    fn plan_keeps_user_changes_and_replaces_untouched_files() {
        let old = [
            t("mods/a.jar", "a1"),
            t("mods/b.jar", "b1"),
            t("mods/gone.jar", "g1"),
            t("mods/edited-gone.jar", "e1"),
            t("config/x.toml", "x1"),
            t("mods/off.jar", "o1"),
            t("mods/deleted.jar", "d1"),
            t("options.txt", "p1"),
        ];
        let new = [
            t("mods/a.jar", "a2"),       // untouched → replace
            t("mods/b.jar", "b1"),       // same in both → same
            t("config/x.toml", "x2"),    // user edited → keep
            t("mods/off.jar", "o2"),     // disabled, untouched → replace disabled
            t("mods/deleted.jar", "d2"), // user deleted → keep deleted
            t("mods/new.jar", "n1"),     // new → add
            t("mods/mine.jar", "m2"),    // user's own file at that path → keep
            t("OPTIONS.TXT", "p2"),      // case-insensitive match → replace
        ];
        let disk = |p: &str| match p {
            "mods/a.jar" => OnDisk::Present("a1".into()),
            "mods/b.jar" => OnDisk::Present("b1".into()),
            "mods/gone.jar" => OnDisk::Present("g1".into()),
            "mods/edited-gone.jar" => OnDisk::Present("changed".into()),
            "config/x.toml" => OnDisk::Present("mine".into()),
            "mods/off.jar" => OnDisk::Disabled("o1".into()),
            "mods/mine.jar" => OnDisk::Present("m1".into()),
            "OPTIONS.TXT" => OnDisk::Present("p1".into()),
            _ => OnDisk::Missing,
        };
        let (steps, track) = plan(&old, &new, &disk);
        let got: HashMap<&str, Action> =
            steps.iter().map(|s| (s.path.as_str(), s.action)).collect();
        let want = [
            ("mods/a.jar", Action::Replace),
            ("mods/b.jar", Action::Same),
            ("config/x.toml", Action::Keep),
            ("mods/off.jar", Action::ReplaceDisabled),
            ("mods/deleted.jar", Action::Keep),
            ("mods/new.jar", Action::Add),
            ("mods/mine.jar", Action::Keep),
            ("OPTIONS.TXT", Action::Replace),
            ("mods/gone.jar", Action::Remove),
            ("mods/edited-gone.jar", Action::Keep),
        ];
        for (p, a) in want {
            assert_eq!(got.get(p), Some(&a), "{p}");
        }
        assert_eq!(steps.len(), want.len());
        let tracked: HashMap<&str, &str> = track
            .iter()
            .map(|t| (t.path.as_str(), t.sha1.as_str()))
            .collect();
        // Kept files keep their install-time hash; removed and foreign ones drop out.
        assert_eq!(tracked.get("config/x.toml"), Some(&"x1"));
        assert_eq!(tracked.get("mods/a.jar"), Some(&"a2"));
        assert_eq!(tracked.get("mods/deleted.jar"), Some(&"d1"));
        assert!(!tracked.contains_key("mods/gone.jar") && !tracked.contains_key("mods/mine.jar"));
    }

    #[test]
    fn apply_moves_staged_files_and_removes_untouched_ones() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        std::fs::create_dir_all(g.join("mods")).unwrap();
        std::fs::create_dir_all(g.join(STAGING)).unwrap();
        std::fs::write(g.join("mods/a.jar"), b"old").unwrap();
        std::fs::write(g.join("mods/gone.jar"), b"g").unwrap();
        std::fs::write(g.join("mods/off.jar.disabled"), b"o").unwrap();
        std::fs::write(g.join(STAGING).join("1"), b"new-a").unwrap();
        std::fs::write(g.join(STAGING).join("2"), b"new-off").unwrap();
        std::fs::write(g.join(STAGING).join("3"), b"cfg").unwrap();
        let step = |p: &str, a| Step {
            path: p.into(),
            action: a,
            new: None,
        };
        let steps = [
            step("mods/a.jar", Action::Replace),
            step("mods/off.jar", Action::ReplaceDisabled),
            step("config/new.toml", Action::Add),
            step("mods/gone.jar", Action::Remove),
            step("mine.txt", Action::Keep),
        ];
        let sources: HashMap<String, PathBuf> = [
            ("mods/a.jar", "1"),
            ("mods/off.jar", "2"),
            ("config/new.toml", "3"),
        ]
        .into_iter()
        .map(|(p, s)| (p.to_owned(), g.join(STAGING).join(s)))
        .collect();
        let r = apply(g, &steps, &sources, vec![]).unwrap();
        assert_eq!((r.added, r.replaced, r.removed, r.kept.len()), (1, 2, 1, 1));
        assert_eq!(std::fs::read(g.join("mods/a.jar")).unwrap(), b"new-a");
        assert_eq!(
            std::fs::read(g.join("mods/off.jar.disabled")).unwrap(),
            b"new-off"
        );
        assert!(!g.join("mods/off.jar").exists());
        assert_eq!(std::fs::read(g.join("config/new.toml")).unwrap(), b"cfg");
        assert!(!g.join("mods/gone.jar").exists());
    }

    #[test]
    fn scan_sees_disabled_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("mods")).unwrap();
        std::fs::write(d.path().join("mods/x.jar.disabled"), b"x").unwrap();
        assert!(matches!(scan(d.path(), "mods/x.jar"), OnDisk::Disabled(_)));
        assert_eq!(scan(d.path(), "mods/y.jar"), OnDisk::Missing);
        assert_eq!(scan(d.path(), "../escape"), OnDisk::Missing);
    }
}
