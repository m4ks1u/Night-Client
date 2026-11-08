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
//! Everything new is first downloaded (verified) or extracted into the
//! update's own staging folder, `.nc-update/<random id>` inside the game
//! folder; only then are files moved into place. A failed download changes
//! nothing.
//!
//! Moving files into place is a transaction:
//! - Every step is checked against the disk first (no file where a folder
//!   is needed, no links, nothing changed since the plan).
//! - A journal of every rename is written before the first change. Old files
//!   are moved into the update folder, never deleted, until the update is
//!   recorded in the database ([`finish`]).
//! - Any failure undoes the renames in reverse. After a crash, [`recover`]
//!   finishes or undoes the update before the next launch or update, so the
//!   files always match what the database records.
//!
//! Only folders that carry Night Client's marker are ever removed from
//! `.nc-update`; anything else there is left alone.

use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};

use nc_core::mrpack::{Pack, PackFile};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::modpack::{self, Tracked};
use crate::{
    LauncherError,
    confined::{self, Confined},
    download::{Downloader, Progress},
    fsutil,
};

/// Staging folder inside the game folder (same volume, so moves are
/// renames). Reserved: packs can't write into it (`nc_core::mrpack`).
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
    /// For replacements and removals: the SHA-1 the file on disk had when
    /// the update was planned. The update verifies the old file still has it
    /// (after moving it aside), so an edit made since is never lost.
    pub expected: Option<String>,
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
        let disk = on_disk(&n.path);
        let action = match (&disk, was) {
            (OnDisk::Missing, None) => Action::Add,
            // The user deleted a file the pack installed: respect that.
            (OnDisk::Missing, Some(_)) => Action::Keep,
            (OnDisk::Present(c), _) if *c == n.sha1 => Action::Same,
            (OnDisk::Present(c), Some(o)) if c == o => Action::Replace,
            (OnDisk::Present(_), _) => Action::Keep,
            (OnDisk::Disabled(c), _) if *c == n.sha1 => Action::Same,
            (OnDisk::Disabled(c), Some(o)) if c == o => Action::ReplaceDisabled,
            (OnDisk::Disabled(_), _) => Action::Keep,
        };
        let expected = match (&disk, action) {
            (
                OnDisk::Present(c) | OnDisk::Disabled(c),
                Action::Replace | Action::ReplaceDisabled,
            ) => Some(c.clone()),
            _ => None,
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
            expected,
        });
    }
    for o in old {
        if seen.contains(&o.path.to_ascii_lowercase()) {
            continue;
        }
        let (action, expected) = match on_disk(&o.path) {
            OnDisk::Missing => continue,
            OnDisk::Present(c) if c == o.sha1 => (Action::Remove, Some(c)),
            OnDisk::Disabled(c) if c == o.sha1 => (Action::RemoveDisabled, Some(c)),
            _ => (Action::Keep, None),
        };
        steps.push(Step {
            path: o.path.clone(),
            action,
            new: None,
            expected,
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
///
/// On success the files are in place and a [`Pending`] is returned. The
/// caller records the update together with [`Pending::id`] in one database
/// transaction (that id is the acknowledgment [`recover`] checks), then calls
/// [`finish`]; if recording fails, it calls [`rollback`]. On error the instance is as it was, unless the error is
/// [`LauncherError::Recovery`]: then undoing failed, and the journal is
/// kept for [`recover`].
#[allow(clippy::too_many_arguments)]
pub async fn update(
    dl: &Downloader,
    game_dir: &Path,
    archive: &Path,
    pack: &Pack,
    old: &[Tracked],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<(UpdateReport, Pending), LauncherError> {
    let root = Confined::new(game_dir).map_err(LauncherError::io(game_dir))?;
    let txn = begin(&root)?;
    let result = stage_and_apply(dl, &root, &txn, archive, pack, old, cancel, on_progress).await;
    match result {
        Ok(report) => Ok((
            report,
            Pending {
                game_dir: game_dir.to_owned(),
                txn,
            },
        )),
        Err(e) => {
            if !matches!(e, LauncherError::Recovery(_)) {
                // Nothing in the instance changed (or it was undone): only
                // this update's own folder goes.
                if let Err(d) = discard(&root, &txn) {
                    tracing::warn!(error = %d, "could not remove the update's staging folder");
                }
            }
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stage_and_apply(
    dl: &Downloader,
    root: &Confined,
    txn: &str,
    archive: &Path,
    pack: &Pack,
    old: &[Tracked],
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<UpdateReport, LauncherError> {
    // 1. Overrides into staging (hashed while copying), then the plan.
    let (arch, r, t) = (archive.to_owned(), root.clone(), txn.to_owned());
    let staged_overrides = tokio::task::spawn_blocking(move || {
        let mut n = 0usize;
        modpack::copy_overrides(&arch, modpack::MAX_OVERRIDES_BYTES, &r, |_| {
            n += 1;
            Ok(format!("{t}/o/{n}"))
        })
    })
    .await
    .map_err(|e| LauncherError::Spawn(e.to_string()))??;
    let overrides: Vec<Tracked> = staged_overrides.iter().map(|(t, _)| t.clone()).collect();
    let new = modpack::tracked_after_install(pack, &overrides);
    let (gd, old_v, new_v) = (root.root().to_owned(), old.to_vec(), new.clone());
    let (steps, tracked) =
        tokio::task::spawn_blocking(move || plan(&old_v, &new_v, &|rel| scan(&gd, rel)))
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))?;

    // 2. Download the pack files that will be written, into staging.
    let staged_override: HashMap<String, String> = staged_overrides
        .into_iter()
        .map(|(t, p)| (t.path.to_ascii_lowercase(), p))
        .collect();
    let files_by: HashMap<String, &PackFile> = pack
        .files
        .iter()
        .map(|f| (f.path.to_ascii_lowercase(), f))
        .collect();
    let mut downloads = Vec::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for s in &steps {
        let Some(n) = &s.new else { continue };
        let key = n.path.to_ascii_lowercase();
        if n.is_override {
            sources.insert(key.clone(), staged_override[&key].clone());
        } else {
            let f = files_by[&key];
            let rel = format!("{txn}/d/{}", downloads.len());
            sources.insert(key, rel.clone());
            downloads.push(PackFile {
                path: rel,
                ..(*f).clone()
            });
        }
    }
    modpack::download_confined(dl, root, &downloads, cancel, on_progress).await?;
    if cancel.is_cancelled() {
        return Err(LauncherError::Cancelled);
    }

    // 3. Apply: everything is staged and verified. Not cancellable from here:
    // it either completes or is undone.
    let (r, t) = (root.clone(), txn.to_owned());
    tokio::task::spawn_blocking(move || apply_with(&r, &t, &steps, &sources, tracked, &|_| false))
        .await
        .map_err(|e| LauncherError::Spawn(e.to_string()))?
}

// ------------------------------------------------------------ transaction ----

/// The file that marks a folder in [`STAGING`] as one Night Client made.
const OWNER: &str = "owner";
const JOURNAL: &str = "journal.json";
/// Written once every change is in place.
const COMMITTED: &str = "committed";
const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
/// Journal layout version. 1 (unreleased) acknowledged by target version.
const JOURNAL_FORMAT: u32 = 2;

fn owner_text(id: &str) -> String {
    format!("Night Client modpack update\n{id}\n")
}

/// An update id: 128 random bits as lower-case hex. It names the update's
/// folder and is what the database records to acknowledge the update.
fn is_txn_id(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn id_of(txn: &str) -> &str {
    txn.rsplit('/').next().unwrap_or_default()
}

/// What an update changes in the instance, written (atomically) before the
/// first change. Every change is a rename that never replaces anything, so
/// undoing needs no other state: each move is reversed where its destination
/// exists and its source doesn't.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    format: u32,
    /// This update's id (its folder name); the database records it with the
    /// update, and only that acknowledgment keeps a committed update.
    id: String,
    /// Folders created in the instance, parents first.
    mkdirs: Vec<String>,
    /// Renames, in order.
    moves: Vec<Move>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Move {
    from: String,
    to: String,
    /// For an old file moved aside: the SHA-1 it must still have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha1: Option<String>,
}

/// Where a test can fail or crash an update (`apply_with`'s `fault`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    /// After the checks, before the journal is written.
    Journal,
    /// Before move `i` (folders are created before move 0).
    Move(usize),
    /// After the last move, before the `committed` marker.
    Commit,
    /// After the `committed` marker.
    Committed,
}

/// An update whose files are in place but not yet recorded. Old files are
/// still kept in the update's own folder until [`finish`].
#[derive(Debug)]
#[must_use = "finish or roll back the update"]
pub struct Pending {
    game_dir: PathBuf,
    txn: String,
}

impl Pending {
    /// The update's id, to record with the update in one database transaction.
    pub fn id(&self) -> &str {
        id_of(&self.txn)
    }
}

/// What [`recover`] found.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovered {
    /// Interrupted updates undone.
    pub rolled_back: usize,
    /// Finished or unstarted updates whose folders were removed.
    pub cleaned: usize,
}

fn io_at(root: &Confined, rel: &str) -> impl FnOnce(std::io::Error) -> LauncherError {
    LauncherError::io(root.root().join(rel))
}

fn conflict(root: &Confined, rel: &str, why: &str) -> LauncherError {
    io_at(root, rel)(std::io::Error::other(why.to_owned()))
}

/// Create this update's own folder, `.nc-update/<random id>`, marked as ours.
/// Anything else in `.nc-update` is left alone.
fn begin(root: &Confined) -> Result<String, LauncherError> {
    root.create_dirs(STAGING).map_err(io_at(root, STAGING))?;
    let id = confined::random_hex(16).map_err(io_at(root, STAGING))?;
    let txn = format!("{STAGING}/{id}");
    let dir = root.path(&txn).map_err(io_at(root, &txn))?;
    // Exclusive: never reuse a folder that is already there.
    std::fs::create_dir(&dir).map_err(LauncherError::io(&dir))?;
    root.write(&format!("{txn}/{OWNER}"), owner_text(&id).as_bytes())
        .map_err(io_at(root, &txn))?;
    Ok(txn)
}

/// True when `txn` is a real folder (not a link) with our marker in it.
fn is_ours(root: &Confined, txn: &str) -> bool {
    let id = id_of(txn);
    if !is_txn_id(id) {
        return false;
    }
    let Ok(dir) = root.path(txn) else {
        return false;
    };
    if !confined::lstat(&dir)
        .ok()
        .flatten()
        .is_some_and(|m| m.is_dir())
    {
        return false;
    }
    let Ok(owner) = root.path(&format!("{txn}/{OWNER}")) else {
        return false;
    };
    let mut buf = Vec::new();
    std::fs::File::open(owner)
        .and_then(|f| f.take(256).read_to_end(&mut buf))
        .is_ok()
        && buf == owner_text(id).as_bytes()
}

/// Remove this update's own folder, and `.nc-update` if that leaves it empty.
/// A folder without our marker is never removed.
fn discard(root: &Confined, txn: &str) -> Result<(), LauncherError> {
    if !is_ours(root, txn) {
        return Err(conflict(root, txn, "not a Night Client update folder"));
    }
    let dir = root.path(txn).map_err(io_at(root, txn))?;
    // `remove_dir_all` does not follow links inside the folder.
    std::fs::remove_dir_all(&dir).map_err(LauncherError::io(&dir))?;
    if let Ok(staging) = root.path(STAGING) {
        let _ = std::fs::remove_dir(staging); // only succeeds when empty
    }
    Ok(())
}

/// Check every step against the disk and build the journal. Changes nothing
/// in the instance (only creates the update's backup folder).
fn prepare(
    root: &Confined,
    txn: &str,
    steps: &[Step],
    sources: &HashMap<String, String>,
    r: &mut UpdateReport,
) -> Result<Journal, LauncherError> {
    let backups = format!("{txn}/b");
    root.create_dirs(&backups).map_err(io_at(root, &backups))?;
    let mut j = Journal {
        format: JOURNAL_FORMAT,
        id: id_of(txn).to_owned(),
        mkdirs: Vec::new(),
        moves: Vec::new(),
    };
    let mut made: HashSet<String> = HashSet::new();
    let backup = |j: &mut Journal, rel: String, s: &Step| -> Result<(), LauncherError> {
        let sha1 = s
            .expected
            .clone()
            .ok_or_else(|| conflict(root, &rel, "no recorded content to check"))?;
        let to = format!("{backups}/{}", j.moves.len());
        j.moves.push(Move {
            from: rel,
            to,
            sha1: Some(sha1),
        });
        Ok(())
    };
    let is_file = |rel: &str| -> Result<bool, LauncherError> {
        let p = root.path(rel).map_err(io_at(root, rel))?;
        Ok(confined::lstat(&p)
            .map_err(LauncherError::io(&p))?
            .is_some_and(|m| m.is_file()))
    };
    for s in steps {
        let rel = match s.action {
            Action::ReplaceDisabled | Action::RemoveDisabled => format!("{}.disabled", s.path),
            _ => s.path.clone(),
        };
        match s.action {
            Action::Same => {
                r.unchanged += 1;
                continue;
            }
            Action::Keep => {
                r.kept.push(s.path.clone());
                continue;
            }
            _ => {}
        }
        // The path must be one a pack may write (also through 8.3 aliases),
        // and every existing folder on the way a real folder, not a link.
        modpack::check_instance_path(root, &s.path)?;
        let dest = root.path(&rel).map_err(io_at(root, &rel))?;
        let present = confined::lstat(&dest).map_err(LauncherError::io(&dest))?;
        match s.action {
            Action::Add | Action::Replace | Action::ReplaceDisabled => {
                let src = &sources[&s.path.to_ascii_lowercase()];
                if !is_file(src)? {
                    return Err(conflict(root, src, "the staged file is missing"));
                }
                let mut prefix = String::new();
                for seg in rel
                    .split('/')
                    .collect::<Vec<_>>()
                    .split_last()
                    .map_or(&[][..], |(_, d)| d)
                {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(seg);
                    let p = root.path(&prefix).map_err(io_at(root, &prefix))?;
                    match confined::lstat(&p).map_err(LauncherError::io(&p))? {
                        Some(m) if m.is_dir() => {}
                        Some(_) => return Err(conflict(root, &prefix, "a file is in the way")),
                        None => {
                            if made.insert(prefix.to_ascii_lowercase()) {
                                j.mkdirs.push(prefix.clone());
                            }
                        }
                    }
                }
                if s.action == Action::Add {
                    if present.is_some() {
                        return Err(conflict(
                            root,
                            &rel,
                            "appeared since the update was planned",
                        ));
                    }
                    r.added += 1;
                } else {
                    if !present.is_some_and(|m| m.is_file()) {
                        return Err(conflict(root, &rel, "changed since the update was planned"));
                    }
                    backup(&mut j, rel.clone(), s)?;
                    r.replaced += 1;
                }
                j.moves.push(Move {
                    from: src.clone(),
                    to: rel,
                    sha1: None,
                });
            }
            Action::Remove | Action::RemoveDisabled => {
                if !present.is_some_and(|m| m.is_file()) {
                    return Err(conflict(root, &rel, "changed since the update was planned"));
                }
                backup(&mut j, rel, s)?;
                r.removed += 1;
            }
            Action::Same | Action::Keep => unreachable!(),
        }
    }
    Ok(j)
}

/// Apply the steps as one transaction. `fault(point)` returning true fails
/// the update there (a test can also crash the process there).
fn apply_with(
    root: &Confined,
    txn: &str,
    steps: &[Step],
    sources: &HashMap<String, String>,
    tracked: Vec<Tracked>,
    fault: &dyn Fn(Point) -> bool,
) -> Result<UpdateReport, LauncherError> {
    let mut r = UpdateReport {
        tracked,
        ..Default::default()
    };
    let j = prepare(root, txn, steps, sources, &mut r)?;
    let jrel = format!("{txn}/{JOURNAL}");
    let body = serde_json::to_vec(&j).map_err(|e| LauncherError::Parse {
        what: jrel.clone(),
        message: e.to_string(),
    })?;
    if fault(Point::Journal) {
        return Err(conflict(root, &jrel, "injected failure"));
    }
    // Nothing has changed until the journal is safely on disk.
    root.write(&jrel, &body).map_err(io_at(root, &jrel))?;
    let injected = |p: Point, rel: &str| {
        if fault(p) {
            Err(conflict(root, rel, "injected failure"))
        } else {
            Ok(())
        }
    };
    let run = (|| -> Result<(), LauncherError> {
        for d in &j.mkdirs {
            root.create_dirs(d).map_err(io_at(root, d))?;
        }
        for (i, m) in j.moves.iter().enumerate() {
            injected(Point::Move(i), &m.to)?;
            // Never replaces: a file that appeared at the destination since
            // the checks makes this fail instead of being overwritten.
            root.rename(&m.from, &m.to).map_err(io_at(root, &m.to))?;
            if let Some(want) = &m.sha1 {
                // The old file is aside now, where nothing else edits it: it
                // must still be what the plan saw, or someone changed it.
                let p = root.path(&m.to).map_err(io_at(root, &m.to))?;
                let got = fsutil::sha1_file(&p).map_err(LauncherError::io(&p))?;
                if !got.eq_ignore_ascii_case(want) {
                    return Err(conflict(
                        root,
                        &m.from,
                        "was changed since the update was planned, so nothing was replaced",
                    ));
                }
            }
        }
        let crel = format!("{txn}/{COMMITTED}");
        injected(Point::Commit, &crel)?;
        root.write(&crel, b"").map_err(io_at(root, &crel))?;
        injected(Point::Committed, &crel)?;
        Ok(())
    })();
    match run {
        Ok(()) => Ok(r),
        Err(e) => match undo(root, txn, &j) {
            Ok(()) => Err(e),
            Err(u) => Err(LauncherError::Recovery(format!(
                "{e}; undoing it also failed ({u}). The old files are kept in {}, and Night Client tries again before the next launch or update.",
                root.root().join(txn).display()
            ))),
        },
    }
}

/// Undo a journal's moves, newest first, then remove the folders it
/// created (when empty). Safe to run again after a partial undo.
///
/// A move is undone when its destination exists and its source doesn't.
/// Anything else means it never happened, or that something else now sits
/// at the source: then nothing is overwritten. Afterwards every old file
/// must be back out of `txn/b`; one that isn't (its place was taken) is an
/// error, and the update folder is kept so it isn't lost.
fn undo(root: &Confined, txn: &str, j: &Journal) -> Result<(), LauncherError> {
    let mut first = None;
    for m in j.moves.iter().rev() {
        let step = || -> Result<(), LauncherError> {
            let to = root.path(&m.to).map_err(io_at(root, &m.to))?;
            let from = root.path(&m.from).map_err(io_at(root, &m.from))?;
            let to_here = confined::lstat(&to)
                .map_err(LauncherError::io(&to))?
                .is_some();
            let from_here = confined::lstat(&from)
                .map_err(LauncherError::io(&from))?
                .is_some();
            if to_here && !from_here {
                root.rename(&m.to, &m.from).map_err(io_at(root, &m.from))?;
            }
            Ok(())
        };
        if let Err(e) = step() {
            first.get_or_insert(e);
        }
    }
    for d in j.mkdirs.iter().rev() {
        if let Ok(p) = root.path(d) {
            let _ = std::fs::remove_dir(p); // only when empty
        }
    }
    if let Some(e) = first {
        return Err(e);
    }
    let backups = format!("{txn}/b/");
    for m in &j.moves {
        if m.to.starts_with(&backups) {
            let p = root.path(&m.to).map_err(io_at(root, &m.to))?;
            if confined::lstat(&p)
                .map_err(LauncherError::io(&p))?
                .is_some()
            {
                return Err(conflict(
                    root,
                    &m.from,
                    "another file is there now, so the old one was not put back",
                ));
            }
        }
    }
    Ok(())
}

fn read_journal(root: &Confined, txn: &str) -> Result<Option<Journal>, LauncherError> {
    let rel = format!("{txn}/{JOURNAL}");
    let p = root.path(&rel).map_err(io_at(root, &rel))?;
    let f = match std::fs::File::open(&p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LauncherError::io(&p)(e)),
    };
    let mut buf = Vec::new();
    f.take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(LauncherError::io(&p))?;
    let parsed = (buf.len() as u64 <= MAX_JOURNAL_BYTES)
        .then(|| serde_json::from_slice(&buf).ok())
        .flatten();
    match parsed {
        Some(j) if is_journal_of(&j, txn) => Ok(Some(j)),
        _ => Err(LauncherError::Recovery(format!(
            "the update journal {} is unreadable or from another version; the update folder is kept",
            p.display()
        ))),
    }
}

fn is_journal_of(j: &Journal, txn: &str) -> bool {
    j.format == JOURNAL_FORMAT && j.id == id_of(txn)
}

/// The update is recorded: drop the kept old files and the update folder.
pub fn finish(p: Pending) -> Result<(), LauncherError> {
    let root = Confined::new(&p.game_dir).map_err(LauncherError::io(&p.game_dir))?;
    discard(&root, &p.txn)
}

/// Recording the update failed (nothing was recorded): put the old files
/// back. Any error is [`LauncherError::Recovery`]: the state is then not
/// confirmed, and [`recover`] tries again before the next launch or update.
pub fn rollback(p: Pending) -> Result<(), LauncherError> {
    let kept = p.game_dir.join(&p.txn);
    let recovery = |e: LauncherError| {
        LauncherError::Recovery(format!(
            "{e}. The old files are kept in {}.",
            kept.display()
        ))
    };
    let root = Confined::new(&p.game_dir)
        .map_err(LauncherError::io(&p.game_dir))
        .map_err(recovery)?;
    let j = read_journal(&root, &p.txn)
        .and_then(|j| j.ok_or_else(|| conflict(&root, &p.txn, "the update journal is missing")))
        .map_err(recovery)?;
    undo(&root, &p.txn, &j).map_err(recovery)?;
    if let Err(e) = discard(&root, &p.txn) {
        // The files are back; the folder is cleaned up by the next recovery.
        tracing::warn!(error = %e, "could not remove the rolled-back update's folder");
    }
    Ok(())
}

/// Finish or undo updates that were interrupted (a crash, power loss, the
/// launcher killed). `acknowledged` is the update id the database recorded
/// for the instance (in the same transaction as the update itself): an
/// update that was committed *and* acknowledged by its own id is kept; any
/// other one is undone, so files and records always match. Running it again
/// is a no-op. Folders in `.nc-update` that Night Client didn't make are
/// never touched. Call it off the async runtime, while no update of this
/// instance runs.
pub fn recover(game_dir: &Path, acknowledged: Option<&str>) -> Result<Recovered, LauncherError> {
    let mut out = Recovered::default();
    let staging = game_dir.join(STAGING);
    match confined::lstat(&staging).map_err(LauncherError::io(&staging))? {
        Some(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        // Missing, or not ours (a file, or a link): nothing to recover.
        _ => return Ok(out),
    }
    let root = Confined::new(game_dir).map_err(LauncherError::io(game_dir))?;
    let entries = std::fs::read_dir(&staging).map_err(LauncherError::io(&staging))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let txn = format!("{STAGING}/{name}");
        if !is_ours(&root, &txn) {
            continue;
        }
        match read_journal(&root, &txn)? {
            // Stopped before changing anything.
            None => {
                discard(&root, &txn)?;
                out.cleaned += 1;
            }
            Some(j) => {
                let crel = format!("{txn}/{COMMITTED}");
                let cp = root.path(&crel).map_err(io_at(&root, &crel))?;
                let committed = confined::lstat(&cp)
                    .map_err(LauncherError::io(&cp))?
                    .is_some();
                if committed && acknowledged == Some(j.id.as_str()) {
                    discard(&root, &txn)?;
                    out.cleaned += 1;
                } else {
                    undo(&root, &txn, &j).map_err(|e| {
                        LauncherError::Recovery(format!(
                            "{e}. The old files are kept in {}.",
                            root.root().join(&txn).display()
                        ))
                    })?;
                    discard(&root, &txn)?;
                    out.rolled_back += 1;
                }
            }
        }
    }
    let _ = std::fs::remove_dir(&staging); // only when empty
    Ok(out)
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

    type Tree = Vec<(String, Option<Vec<u8>>)>;

    /// Every file and folder under `g` except `.nc-update`, with contents.
    fn snapshot(g: &Path) -> Tree {
        let mut out = Vec::new();
        let mut stack = vec![(g.to_path_buf(), String::new())];
        while let Some((dir, rel)) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if rel.is_empty() && name == STAGING {
                    continue;
                }
                let r = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                if e.file_type().unwrap().is_dir() {
                    out.push((r.clone(), None));
                    stack.push((e.path(), r));
                } else {
                    out.push((r, Some(std::fs::read(e.path()).unwrap())));
                }
            }
        }
        out.sort();
        out
    }

    fn h(b: &[u8]) -> String {
        hex::encode(<sha1::Sha1 as sha1::Digest>::digest(b))
    }

    fn step(p: &str, a: Action, expected: Option<&[u8]>) -> Step {
        Step {
            path: p.into(),
            action: a,
            new: None,
            expected: expected.map(h),
        }
    }

    /// An instance with a file to replace, a disabled file to replace, a new
    /// file in a new folder, a file to remove and a user file; and the staged
    /// sources. Returns the root, the update folder, steps and sources.
    fn fixture(g: &Path) -> (Confined, String, Vec<Step>, HashMap<String, String>) {
        std::fs::create_dir_all(g.join("mods")).unwrap();
        std::fs::write(g.join("mods/a.jar"), b"old").unwrap();
        std::fs::write(g.join("mods/gone.jar"), b"g").unwrap();
        std::fs::write(g.join("mods/off.jar.disabled"), b"o").unwrap();
        std::fs::write(g.join("mine.txt"), b"user").unwrap();
        let c = Confined::new(g).unwrap();
        let txn = begin(&c).unwrap();
        let mut sources = HashMap::new();
        for (p, n, body) in [
            ("mods/a.jar", "1", &b"new-a"[..]),
            ("mods/off.jar", "2", b"new-off"),
            ("config/deep/new.toml", "3", b"cfg"),
        ] {
            let rel = format!("{txn}/o/{n}");
            c.write(&rel, body).unwrap();
            sources.insert(p.to_owned(), rel);
        }
        let steps = vec![
            step("mods/a.jar", Action::Replace, Some(b"old")),
            step("mods/off.jar", Action::ReplaceDisabled, Some(b"o")),
            step("config/deep/new.toml", Action::Add, None),
            step("mods/gone.jar", Action::Remove, Some(b"g")),
            step("mine.txt", Action::Keep, None),
        ];
        (c, txn, steps, sources)
    }

    /// Journal order: a.jar aside (0) and in (1), off.jar aside (2) and in
    /// (3), the new config (4), gone.jar aside (5).
    const MOVES: usize = 6;
    const ADD_MOVE: usize = 4;

    fn never(_: Point) -> bool {
        false
    }

    /// The fixture before and after a successful update.
    fn before_and_after() -> (Tree, Tree) {
        let d = tempfile::tempdir().unwrap();
        let (c, txn, steps, sources) = fixture(d.path());
        let before = snapshot(d.path());
        apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap();
        (before, snapshot(d.path()))
    }

    fn pending(g: &Path, txn: &str) -> Pending {
        Pending {
            game_dir: g.to_owned(),
            txn: txn.to_owned(),
        }
    }

    #[test]
    fn apply_moves_staged_files_and_keeps_old_ones_until_finished() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let r = apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap();
        assert_eq!((r.added, r.replaced, r.removed, r.kept.len()), (1, 2, 1, 1));
        assert_eq!(std::fs::read(g.join("mods/a.jar")).unwrap(), b"new-a");
        assert_eq!(
            std::fs::read(g.join("mods/off.jar.disabled")).unwrap(),
            b"new-off"
        );
        assert!(!g.join("mods/off.jar").exists());
        assert_eq!(
            std::fs::read(g.join("config/deep/new.toml")).unwrap(),
            b"cfg"
        );
        assert!(!g.join("mods/gone.jar").exists());
        assert_eq!(std::fs::read(g.join("mine.txt")).unwrap(), b"user");
        let j = read_journal(&c, &txn).unwrap().unwrap();
        assert_eq!(
            (j.moves.len(), j.format, j.id.as_str()),
            (MOVES, 2, id_of(&txn))
        );
        assert_eq!(j.moves[ADD_MOVE].to, "config/deep/new.toml");
        finish(pending(g, &txn)).unwrap();
        assert!(!g.join(STAGING).exists(), "staging removed once empty");
    }

    #[test]
    fn update_ids_are_unique_and_name_the_folder() {
        let d = tempfile::tempdir().unwrap();
        let c = Confined::new(d.path()).unwrap();
        let (a, b) = (begin(&c).unwrap(), begin(&c).unwrap());
        assert_ne!(a, b);
        for t in [&a, &b] {
            assert!(is_txn_id(id_of(t)) && pending(d.path(), t).id() == id_of(t));
        }
    }

    fn all_points() -> Vec<Point> {
        let mut v = vec![Point::Journal];
        v.extend((0..MOVES).map(Point::Move));
        v.extend([Point::Commit, Point::Committed]);
        v
    }

    #[test]
    fn a_failure_at_any_point_restores_the_instance() {
        for at in all_points() {
            let d = tempfile::tempdir().unwrap();
            let g = d.path();
            let (c, txn, steps, sources) = fixture(g);
            let before = snapshot(g);
            let e = apply_with(&c, &txn, &steps, &sources, vec![], &|p| p == at);
            assert!(matches!(e, Err(LauncherError::Io { .. })), "{at:?}: {e:?}");
            assert_eq!(snapshot(g), before, "{at:?}");
            discard(&c, &txn).unwrap();
            assert!(!g.join(STAGING).exists());
        }
    }

    #[test]
    fn preflight_refuses_a_file_in_the_way_before_changing_anything() {
        // The audit's case: the first file would be replaced, then a later
        // one needs a folder where a file is.
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, mut steps, mut sources) = fixture(g);
        std::fs::write(g.join("blocker"), b"a file").unwrap();
        let rel = format!("{txn}/o/9");
        c.write(&rel, b"b").unwrap();
        sources.insert("blocker/b.txt".into(), rel);
        steps.push(step("blocker/b.txt", Action::Add, None));
        let before = snapshot(g);
        let e = apply_with(&c, &txn, &steps, &sources, vec![], &never);
        let e = e.unwrap_err().to_string();
        assert!(
            e.contains("blocker") && e.contains("folder is needed"),
            "{e}"
        );
        assert_eq!(snapshot(g), before);
        assert!(read_journal(&c, &txn).unwrap().is_none(), "nothing started");
    }

    #[test]
    fn preflight_refuses_a_file_that_appeared_at_an_added_path() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        std::fs::create_dir_all(g.join("config/deep")).unwrap();
        std::fs::write(g.join("config/deep/new.toml"), b"appeared").unwrap();
        let before = snapshot(g);
        assert!(apply_with(&c, &txn, &steps, &sources, vec![], &never).is_err());
        assert_eq!(snapshot(g), before);
    }

    /// Re-review RR-003: a file edited after the plan (while downloading) is
    /// neither replaced nor removed, for every kind of step.
    #[test]
    fn an_edit_made_after_planning_is_never_lost() {
        for edited in ["mods/a.jar", "mods/off.jar.disabled", "mods/gone.jar"] {
            let d = tempfile::tempdir().unwrap();
            let g = d.path();
            let (c, txn, steps, sources) = fixture(g);
            std::fs::write(g.join(edited), b"user edit during download").unwrap();
            let before = snapshot(g);
            let e = apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap_err();
            assert!(e.to_string().contains("changed since"), "{edited}: {e}");
            assert_eq!(
                snapshot(g),
                before,
                "{edited}: the edit and everything else kept"
            );
            discard(&c, &txn).unwrap();
        }
    }

    /// An edit between the checks and the move itself is caught too: the old
    /// file is verified after it has been moved aside.
    #[test]
    fn an_edit_just_before_a_move_is_caught_and_undone() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let path = g.join("mods/a.jar");
        let r = apply_with(&c, &txn, &steps, &sources, vec![], &|p| {
            if p == Point::Move(0) {
                std::fs::write(&path, b"edited at the last moment").unwrap();
            }
            false
        });
        assert!(r.unwrap_err().to_string().contains("changed since"));
        assert_eq!(std::fs::read(&path).unwrap(), b"edited at the last moment");
        assert_eq!(std::fs::read(g.join("mods/gone.jar")).unwrap(), b"g");
    }

    /// A file that appears at an added path between the checks and the move
    /// is not overwritten: the move refuses to replace it.
    #[test]
    fn a_file_appearing_just_before_its_move_is_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let path = g.join("config/deep/new.toml");
        let r = apply_with(&c, &txn, &steps, &sources, vec![], &|p| {
            if p == Point::Move(ADD_MOVE) {
                std::fs::write(&path, b"made by the user").unwrap();
            }
            false
        });
        assert!(r.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"made by the user");
        assert_eq!(std::fs::read(g.join("mods/a.jar")).unwrap(), b"old");
    }

    #[test]
    fn rollback_after_success_restores_the_old_files() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let before = snapshot(g);
        apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap();
        rollback(pending(g, &txn)).unwrap();
        assert_eq!(snapshot(g), before);
        assert!(!g.join(STAGING).exists());
    }

    /// Leave an update as a crash would, with only the first `done` moves
    /// applied (all of them: `MOVES`). Returns the tree before and the id.
    fn rewound_to(g: &Path, done: usize, committed: bool) -> (Tree, String) {
        let (c, txn, steps, sources) = fixture(g);
        let before = snapshot(g);
        apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap();
        let j = read_journal(&c, &txn).unwrap().unwrap();
        for m in j.moves[done..].iter().rev() {
            c.rename(&m.to, &m.from).unwrap();
        }
        if !committed {
            std::fs::remove_file(c.path(&format!("{txn}/{COMMITTED}")).unwrap()).unwrap();
        }
        (before, id_of(&txn).to_owned())
    }

    #[test]
    fn recover_undoes_an_uncommitted_update_after_any_number_of_moves() {
        for done in 0..=MOVES {
            let d = tempfile::tempdir().unwrap();
            let g = d.path();
            let (before, id) = rewound_to(g, done, false);
            // Not committed: undone even if (wrongly) acknowledged.
            let r = recover(g, Some(&id)).unwrap();
            assert_eq!(
                r,
                Recovered {
                    rolled_back: 1,
                    cleaned: 0
                },
                "after {done}"
            );
            assert_eq!(snapshot(g), before, "after {done}");
            assert!(!g.join(STAGING).exists());
        }
    }

    /// Re-review RR-004: only the update's own id acknowledges it. Another
    /// id (an earlier update, maybe to the same version) or none undoes it.
    #[test]
    fn recover_keeps_only_an_update_acknowledged_by_its_own_id() {
        let (_, after) = before_and_after();
        let d = tempfile::tempdir().unwrap();
        let (_, id) = rewound_to(d.path(), MOVES, true);
        assert_eq!(recover(d.path(), Some(&id)).unwrap().cleaned, 1);
        assert_eq!(snapshot(d.path()), after, "acknowledged: new files stay");
        assert!(!d.path().join(STAGING).exists());

        let earlier = "0".repeat(32);
        for ack in [None, Some(earlier.as_str())] {
            let d = tempfile::tempdir().unwrap();
            let (before, _) = rewound_to(d.path(), MOVES, true);
            assert_eq!(recover(d.path(), ack).unwrap().rolled_back, 1, "{ack:?}");
            assert_eq!(snapshot(d.path()), before, "{ack:?}: old files back");
            // Running it again finds nothing.
            assert_eq!(recover(d.path(), ack).unwrap(), Recovered::default());
        }
    }

    #[test]
    fn recover_never_overwrites_a_file_that_appeared_and_keeps_the_backup() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        rewound_to(g, MOVES, false);
        // The removed file's path is taken by a new file before recovery.
        std::fs::write(g.join("mods/gone.jar"), b"new user file").unwrap();
        let e = recover(g, None).unwrap_err();
        assert!(matches!(e, LauncherError::Recovery(_)), "{e}");
        assert_eq!(
            std::fs::read(g.join("mods/gone.jar")).unwrap(),
            b"new user file"
        );
        // Everything else was restored, and the old file is still kept.
        assert_eq!(std::fs::read(g.join("mods/a.jar")).unwrap(), b"old");
        let kept = std::fs::read_dir(g.join(STAGING)).unwrap().count();
        assert_eq!(kept, 1, "update folder kept for a later try");
    }

    #[test]
    fn a_journal_from_another_format_or_folder_is_kept_not_trusted() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        apply_with(&c, &txn, &steps, &sources, vec![], &never).unwrap();
        let jrel = format!("{txn}/{JOURNAL}");
        let mut j = read_journal(&c, &txn).unwrap().unwrap();
        j.id = "f".repeat(32);
        c.write(&jrel, &serde_json::to_vec(&j).unwrap()).unwrap();
        let after = snapshot(g);
        assert!(matches!(recover(g, None), Err(LauncherError::Recovery(_))));
        assert_eq!(snapshot(g), after, "nothing undone on an untrusted journal");
        assert!(g.join(&txn).exists());
    }

    // ---- real crashes: a child process aborts at a point, then recovery ----

    const CRASH_ENV: &str = "NC_TEST_PACKUPDATE_CRASH";

    fn point_named(name: &str) -> Option<Point> {
        match name {
            "journal" => Some(Point::Journal),
            "commit" => Some(Point::Commit),
            "committed" => Some(Point::Committed),
            "end" => None,
            m => Some(Point::Move(
                m.strip_prefix("move").unwrap().parse().unwrap(),
            )),
        }
    }

    /// Runs only as the child of `a_real_crash_at_any_point_is_recovered`:
    /// sets up the fixture, records the update id, and aborts the process
    /// (no unwinding, no undo, no destructors) at the requested point.
    #[test]
    fn crash_child() {
        let Ok(spec) = std::env::var(CRASH_ENV) else {
            return;
        };
        let mut it = spec.split('|');
        let (g, id_file, name) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        let g = Path::new(g);
        let (c, txn, steps, sources) = fixture(g);
        std::fs::write(id_file, id_of(&txn)).unwrap();
        let at = point_named(name);
        apply_with(&c, &txn, &steps, &sources, vec![], &|p| {
            if Some(p) == at {
                std::process::abort();
            }
            false
        })
        .unwrap();
        // "end": the files are committed; the database write never happens.
        std::process::abort();
    }

    #[test]
    fn a_real_crash_at_any_point_is_recovered() {
        if std::env::var(CRASH_ENV).is_ok() {
            return;
        }
        let (before, after) = before_and_after();
        let exe = std::env::current_exe().unwrap();
        let mut names = vec!["journal".to_owned()];
        names.extend((0..MOVES).map(|i| format!("move{i}")));
        names.extend(["commit", "committed", "end"].map(String::from));
        for name in &names {
            for acknowledged in [false, true] {
                let d = tempfile::tempdir().unwrap();
                let g = d.path().join("game");
                std::fs::create_dir(&g).unwrap();
                let id_file = d.path().join("id");
                let out = std::process::Command::new(&exe)
                    .args([
                        "--exact",
                        "modrinth::packupdate::tests::crash_child",
                        "--test-threads=1",
                        "--nocapture",
                    ])
                    .env(
                        CRASH_ENV,
                        format!("{}|{}|{name}", g.display(), id_file.display()),
                    )
                    .output()
                    .unwrap();
                // An abort, not a test failure (exit code 101, a panic message).
                let text =
                    String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
                assert!(
                    !out.status.success()
                        && out.status.code() != Some(101)
                        && !text.contains("panicked"),
                    "{name}: the child must have aborted: {:?}\n{text}",
                    out.status
                );
                let id = std::fs::read_to_string(&id_file).unwrap();
                // The database acknowledges the update only after it was
                // committed; an acknowledgment without the marker is also
                // tested, and must not keep anything.
                let ack = acknowledged.then_some(id.as_str());
                recover(&g, ack).unwrap();
                let kept = acknowledged && matches!(name.as_str(), "committed" | "end");
                let want = if kept { &after } else { &before };
                assert_eq!(&snapshot(&g), want, "{name}, acknowledged: {acknowledged}");
                assert!(!g.join(STAGING).exists(), "{name}: update folder removed");
                assert_eq!(
                    recover(&g, ack).unwrap(),
                    Recovered::default(),
                    "{name}: idempotent"
                );
            }
        }
    }

    #[test]
    fn foreign_content_in_staging_is_never_removed() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let staging = g.join(STAGING);
        let foreign = "0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(staging.join(foreign)).unwrap();
        std::fs::write(staging.join("user.txt"), b"user").unwrap();
        std::fs::write(staging.join(foreign).join("owner"), b"someone else").unwrap();
        let c = Confined::new(g).unwrap();
        let txn = begin(&c).unwrap();
        assert!(discard(&c, &format!("{STAGING}/{foreign}")).is_err());
        discard(&c, &txn).unwrap();
        assert_eq!(recover(g, None).unwrap(), Recovered::default());
        assert_eq!(std::fs::read(staging.join("user.txt")).unwrap(), b"user");
        assert!(staging.join(foreign).join("owner").exists());
    }

    #[test]
    fn staging_that_is_a_file_is_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(STAGING), b"user file").unwrap();
        let c = Confined::new(d.path()).unwrap();
        assert!(begin(&c).is_err());
        assert_eq!(recover(d.path(), None).unwrap(), Recovered::default());
        assert_eq!(std::fs::read(d.path().join(STAGING)).unwrap(), b"user file");
    }

    #[test]
    fn staging_is_reserved_for_packs() {
        assert!(!nc_core::mrpack::instance_path_allowed(&format!(
            "{STAGING}/x"
        )));
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_in_the_instance_stops_the_update_before_any_change() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        let g = d.path();
        let (c, txn, mut steps, mut sources) = fixture(g);
        confined::make_junction(&g.join("linked"), outside.path());
        let rel = format!("{txn}/o/9");
        c.write(&rel, b"x").unwrap();
        sources.insert("linked/probe.txt".into(), rel);
        steps.push(step("linked/probe.txt", Action::Add, None));
        let a = std::fs::read(g.join("mods/a.jar")).unwrap();
        assert!(apply_with(&c, &txn, &steps, &sources, vec![], &never).is_err());
        assert_eq!(std::fs::read(g.join("mods/a.jar")).unwrap(), a);
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
        std::fs::remove_dir(g.join("linked")).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_at_the_staging_folder_is_refused_and_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        confined::make_junction(&d.path().join(STAGING), outside.path());
        let c = Confined::new(d.path()).unwrap();
        assert!(begin(&c).is_err());
        assert_eq!(recover(d.path(), None).unwrap(), Recovered::default());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
        std::fs::remove_dir(d.path().join(STAGING)).unwrap();
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
