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
/// as `pack`). `old` is what the previous version left. `target` is what the
/// caller records once the update is in place (the Modrinth version id);
/// [`recover`] compares it after a crash.
///
/// On success the files are in place and a [`Pending`] is returned: call
/// [`finish`] once the update is recorded, or [`rollback`] if recording
/// fails. On error the instance is as it was, unless the error is
/// [`LauncherError::Recovery`]: then undoing failed, and the journal is
/// kept for [`recover`].
#[allow(clippy::too_many_arguments)]
pub async fn update(
    dl: &Downloader,
    game_dir: &Path,
    archive: &Path,
    pack: &Pack,
    old: &[Tracked],
    target: &str,
    cancel: &CancellationToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<(UpdateReport, Pending), LauncherError> {
    let root = Confined::new(game_dir).map_err(LauncherError::io(game_dir))?;
    let txn = begin(&root)?;
    let result = stage_and_apply(
        dl,
        &root,
        &txn,
        archive,
        pack,
        old,
        target,
        cancel,
        on_progress,
    )
    .await;
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
    target: &str,
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
    let (r, t, target) = (root.clone(), txn.to_owned(), target.to_owned());
    tokio::task::spawn_blocking(move || {
        apply_with(&r, &t, &steps, &sources, tracked, &target, &|_| false)
    })
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

fn owner_text(id: &str) -> String {
    format!("Night Client modpack update\n{id}\n")
}

fn is_txn_id(name: &str) -> bool {
    name.len() == 16 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What an update changes in the instance, written (atomically) before the
/// first change. Every change is a rename, so undoing needs no other state:
/// each move is reversed where its destination exists and its source
/// doesn't.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    /// What the caller records when the update is in place.
    target: String,
    /// Folders created in the instance, parents first.
    mkdirs: Vec<String>,
    /// Renames, in order.
    moves: Vec<Move>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Move {
    from: String,
    to: String,
}

/// An update whose files are in place but not yet recorded. Old files are
/// still kept in the update's own folder until [`finish`].
#[derive(Debug)]
#[must_use = "finish or roll back the update"]
pub struct Pending {
    game_dir: PathBuf,
    txn: String,
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
    let id = confined::random_hex(8).map_err(io_at(root, STAGING))?;
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
    let id = txn.rsplit('/').next().unwrap_or_default();
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
    target: &str,
    r: &mut UpdateReport,
) -> Result<Journal, LauncherError> {
    let backups = format!("{txn}/b");
    root.create_dirs(&backups).map_err(io_at(root, &backups))?;
    let mut j = Journal {
        target: target.to_owned(),
        mkdirs: Vec::new(),
        moves: Vec::new(),
    };
    let mut made: HashSet<String> = HashSet::new();
    let backup = |j: &mut Journal, rel: String| {
        let to = format!("{backups}/{}", j.moves.len());
        j.moves.push(Move { from: rel, to });
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
                    backup(&mut j, rel.clone());
                    r.replaced += 1;
                }
                j.moves.push(Move {
                    from: src.clone(),
                    to: rel,
                });
            }
            Action::Remove | Action::RemoveDisabled => {
                if !present.is_some_and(|m| m.is_file()) {
                    return Err(conflict(root, &rel, "changed since the update was planned"));
                }
                backup(&mut j, rel);
                r.removed += 1;
            }
            Action::Same | Action::Keep => unreachable!(),
        }
    }
    Ok(j)
}

/// Apply the steps as one transaction. `fault(i)` fails before move `i`
/// (`i == moves.len()`: before committing); for tests.
fn apply_with(
    root: &Confined,
    txn: &str,
    steps: &[Step],
    sources: &HashMap<String, String>,
    tracked: Vec<Tracked>,
    target: &str,
    fault: &dyn Fn(usize) -> bool,
) -> Result<UpdateReport, LauncherError> {
    let mut r = UpdateReport {
        tracked,
        ..Default::default()
    };
    let j = prepare(root, txn, steps, sources, target, &mut r)?;
    let jrel = format!("{txn}/{JOURNAL}");
    let body = serde_json::to_vec(&j).map_err(|e| LauncherError::Parse {
        what: jrel.clone(),
        message: e.to_string(),
    })?;
    // Nothing has changed until the journal is safely on disk.
    root.write(&jrel, &body).map_err(io_at(root, &jrel))?;
    let injected = |i: usize, rel: &str| {
        if fault(i) {
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
            injected(i, &m.to)?;
            root.rename(&m.from, &m.to).map_err(io_at(root, &m.to))?;
        }
        let crel = format!("{txn}/{COMMITTED}");
        injected(j.moves.len(), &crel)?;
        root.write(&crel, b"").map_err(io_at(root, &crel))?;
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
    parsed.map(Some).ok_or_else(|| LauncherError::Recovery(format!(
        "the update journal {} is unreadable; nothing was changed, and the update folder is kept",
        p.display()
    )))
}

/// The update is recorded: drop the kept old files and the update folder.
pub fn finish(p: Pending) -> Result<(), LauncherError> {
    let root = Confined::new(&p.game_dir).map_err(LauncherError::io(&p.game_dir))?;
    discard(&root, &p.txn)
}

/// Recording the update failed: put the old files back.
pub fn rollback(p: Pending) -> Result<(), LauncherError> {
    let root = Confined::new(&p.game_dir).map_err(LauncherError::io(&p.game_dir))?;
    let j = read_journal(&root, &p.txn)?
        .ok_or_else(|| conflict(&root, &p.txn, "the update journal is missing"))?;
    undo(&root, &p.txn, &j).map_err(|e| {
        LauncherError::Recovery(format!(
            "{e}. The old files are kept in {}.",
            root.root().join(&p.txn).display()
        ))
    })?;
    discard(&root, &p.txn)
}

/// Finish or undo updates that were interrupted (a crash, power loss, the
/// launcher killed). `recorded` is what the database says the instance is
/// at now: an update that was committed *and* recorded is kept; any other
/// one is undone, so files and records always match. Folders in
/// `.nc-update` that Night Client didn't make are never touched. Call it
/// off the async runtime, while no update of this instance runs.
pub fn recover(game_dir: &Path, recorded: Option<&str>) -> Result<Recovered, LauncherError> {
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
                if committed && recorded == Some(j.target.as_str()) {
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

    /// Every file and folder under `g` except `.nc-update`, with contents.
    fn snapshot(g: &Path) -> Vec<(String, Option<Vec<u8>>)> {
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

    fn step(p: &str, a: Action) -> Step {
        Step {
            path: p.into(),
            action: a,
            new: None,
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
            step("mods/a.jar", Action::Replace),
            step("mods/off.jar", Action::ReplaceDisabled),
            step("config/deep/new.toml", Action::Add),
            step("mods/gone.jar", Action::Remove),
            step("mine.txt", Action::Keep),
        ];
        (c, txn, steps, sources)
    }

    const MOVES: usize = 6; // 2 replaces (backup + move), 1 add, 1 removal

    #[test]
    fn apply_moves_staged_files_and_keeps_old_ones_until_finished() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let r = apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false).unwrap();
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
        assert_eq!(read_journal(&c, &txn).unwrap().unwrap().moves.len(), MOVES);
        finish(Pending {
            game_dir: g.to_owned(),
            txn,
        })
        .unwrap();
        assert!(!g.join(STAGING).exists(), "staging removed once empty");
    }

    #[test]
    fn a_failure_at_any_step_restores_the_instance() {
        for at in 0..=MOVES {
            let d = tempfile::tempdir().unwrap();
            let g = d.path();
            let (c, txn, steps, sources) = fixture(g);
            let before = snapshot(g);
            let e = apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|i| i == at);
            assert!(
                matches!(e, Err(LauncherError::Io { .. })),
                "step {at}: {e:?}"
            );
            assert_eq!(snapshot(g), before, "step {at}");
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
        steps.push(step("blocker/b.txt", Action::Add));
        let before = snapshot(g);
        let e = apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false);
        let e = e.unwrap_err().to_string();
        assert!(
            e.contains("blocker") && e.contains("folder is needed"),
            "{e}"
        );
        assert_eq!(snapshot(g), before);
        assert!(
            read_journal(&c, &txn).unwrap().is_none(),
            "no journal: nothing started"
        );
    }

    #[test]
    fn preflight_refuses_files_changed_since_the_plan() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        std::fs::create_dir_all(g.join("config/deep")).unwrap();
        std::fs::write(g.join("config/deep/new.toml"), b"appeared").unwrap();
        let before = snapshot(g);
        assert!(apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false).is_err());
        assert_eq!(snapshot(g), before);
    }

    #[test]
    fn rollback_after_success_restores_the_old_files() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let (c, txn, steps, sources) = fixture(g);
        let before = snapshot(g);
        apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false).unwrap();
        rollback(Pending {
            game_dir: g.to_owned(),
            txn,
        })
        .unwrap();
        assert_eq!(snapshot(g), before);
        assert!(!g.join(STAGING).exists());
    }

    /// Leave a committed update as a crash would, with only the first
    /// `done` moves applied (all of them: `MOVES`).
    fn crashed_after(g: &Path, done: usize, committed: bool) -> Vec<(String, Option<Vec<u8>>)> {
        let (c, txn, steps, sources) = fixture(g);
        let before = snapshot(g);
        apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false).unwrap();
        let j = read_journal(&c, &txn).unwrap().unwrap();
        for m in j.moves[done..].iter().rev() {
            c.rename(&m.to, &m.from).unwrap();
        }
        if !committed {
            std::fs::remove_file(c.path(&format!("{txn}/{COMMITTED}")).unwrap()).unwrap();
        }
        before
    }

    #[test]
    fn recover_undoes_an_interrupted_update_at_every_point() {
        for done in 0..=MOVES {
            let d = tempfile::tempdir().unwrap();
            let g = d.path();
            let before = crashed_after(g, done, false);
            // Not committed: undone whatever the database says.
            let r = recover(g, Some("v2")).unwrap();
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

    #[test]
    fn recover_keeps_a_recorded_update_and_undoes_an_unrecorded_one() {
        let d = tempfile::tempdir().unwrap();
        crashed_after(d.path(), MOVES, true);
        let after = snapshot(d.path());
        assert_eq!(recover(d.path(), Some("v2")).unwrap().cleaned, 1);
        assert_eq!(snapshot(d.path()), after, "recorded: new files stay");
        assert!(!d.path().join(STAGING).exists());

        let d = tempfile::tempdir().unwrap();
        let before = crashed_after(d.path(), MOVES, true);
        assert_eq!(recover(d.path(), Some("v1")).unwrap().rolled_back, 1);
        assert_eq!(snapshot(d.path()), before, "not recorded: old files back");
        // Running it again finds nothing.
        assert_eq!(recover(d.path(), Some("v1")).unwrap(), Recovered::default());
    }

    #[test]
    fn recover_never_overwrites_a_file_that_appeared_and_keeps_the_backup() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        crashed_after(g, MOVES, false);
        // The removed file's path is taken by a new file before recovery.
        std::fs::write(g.join("mods/gone.jar"), b"new user file").unwrap();
        let e = recover(g, Some("v1")).unwrap_err();
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
    fn foreign_content_in_staging_is_never_removed() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        let staging = g.join(STAGING);
        std::fs::create_dir_all(staging.join("0123456789abcdef")).unwrap();
        std::fs::write(staging.join("user.txt"), b"user").unwrap();
        std::fs::write(staging.join("0123456789abcdef/owner"), b"someone else").unwrap();
        let c = Confined::new(g).unwrap();
        let txn = begin(&c).unwrap();
        assert!(discard(&c, &format!("{STAGING}/0123456789abcdef")).is_err());
        discard(&c, &txn).unwrap();
        assert_eq!(recover(g, None).unwrap(), Recovered::default());
        assert_eq!(std::fs::read(staging.join("user.txt")).unwrap(), b"user");
        assert!(staging.join("0123456789abcdef/owner").exists());
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
        steps.push(step("linked/probe.txt", Action::Add));
        let a = std::fs::read(g.join("mods/a.jar")).unwrap();
        assert!(apply_with(&c, &txn, &steps, &sources, vec![], "v2", &|_| false).is_err());
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
