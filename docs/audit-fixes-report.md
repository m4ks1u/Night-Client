# Audit fixes report: NC-AUDIT-001 to 003

Branch: `fix/audit-001-003` (local, branched from `feat/stage-15`, not pushed or merged) · Date:
2026-11-08 · Audit: independent reviewer, `review/initial-engineering-review` @ `55a8d65`,
`docs/independent-initial-audit.md`

I fixed the three confirmed findings as narrow patches, with regression tests for the fixed behavior.
**Nothing has been pushed, merged or published, and the independent reviewer worktree was not modified.** No new
crates, no new dependencies and no database migration.

## Findings: cause and fix

### NC-AUDIT-001 (High): writes could follow an existing junction out of the target folder

**Cause (confirmed).**
- `fsutil::safe_join` only checks the path as text.
- `extract_zip`, `copy_overrides`/`extract_overrides`, modpack downloads and the update's renames then
  called `create_dir_all`, `File::create` and `rename`. The OS follows a junction or symlink that
  already exists under the root.
- Temporary files had guessable names (`.<pid>.tmp`, `.nc-tmp`) and were opened without `create_new`.

**Fix.** A new module, `nc_launcher::confined::Confined`:
- **Links refused.** Every existing component below the root is checked with `symlink_metadata`.
  Symlinks and junctions are refused. Windows reports both as `is_symlink` because they are
  name-surrogate reparse points. Other reparse points, like OneDrive placeholders, still work as normal
  files.
- **Folders one level at a time.** Folders are created one level at a time and each is checked again.
- **Exclusive temp files.** Temporary files get a random name from the OS RNG and are created with
  `create_new`, so a pre-planted file or link is never opened.
- **Second check before the rename.** Right before each rename, the destination is checked again and its
  folder is resolved with `canonicalize`. It must still be inside the root, which catches a folder swapped
  for a junction after the first check.
- **The root itself is trusted.** It was chosen by the user or the launcher, and it may be a junction,
  for example a moved instance folder.

**Where it's used:**
- `extract_zip` (natives)
- modpack downloads (the destination and its `.part` file)
- overrides on import and update
- every rename and backup of an update

`atomic_write` and the game-settings writes (`options.txt`, `iris.properties`) also use random,
exclusive temporary files now.

### NC-AUDIT-002 (High): a failed update could leave files half replaced

**Cause (confirmed).**
- `packupdate::apply` renamed and deleted files one by one, with no preflight, no backups and no undo.
- `update()` then deleted the staging folder, which removed the only copy of the old state.
- The command reported any cancelled failure as "Nothing was changed".
- The database was written after the files, with nothing to reconcile the two after a crash.

**Fix:** the update is a transaction (`packupdate.rs`):
1. **Preflight.** Every step is checked before any change: no file where a folder is needed (the audit's
   `blocker` case), no links, no file that appeared or changed since the plan, and the staged sources
   are present.
2. **Journal.** `journal.json` (every folder to create and every rename) is written atomically and
   flushed before the first change.
3. **Old files are kept, not deleted.** They are renamed into the update's folder (`b/`).
4. **Commit marker.** A `committed` marker is written after the last rename.
5. **Undo on any failure.** Renames are undone in reverse. A move is undone only where its destination
   exists and its source doesn't, so undo is idempotent and can run again after a crash. It never
   overwrites. If an old file can't go back because its place was taken, that is an error, and the
   folder with the old file is kept.
6. **Tied to the database.** `update()` returns a `Pending`. The command records the new version, then
   calls `finish`, which drops the old files. If recording fails, `rollback` puts them back.
7. **Recovery after a crash, power loss or kill.** `packupdate::recover` runs before every update and
   before every launch:
   - our folder with no journal (nothing was changed): removed;
   - committed, and the database already has the target version: finished;
   - anything else: undone.

   Files and database records therefore always match.
8. **Cancellation.**
   - Cancelling still works until the files start moving.
   - From then on, the update either completes or is undone.
   - "Update cancelled. Nothing was changed." is shown only when that is true. A failed undo has its own
     error code (`update_recovery`), and its message says where the old files are.

### NC-AUDIT-003 (Medium): the update deleted whatever was in `.nc-update`

**Cause (confirmed).**
- `update()` ran `remove_dir_all(".nc-update")` before and after every update, without checking what was
  inside.
- `instance_path_allowed` reserved only `.natives`, so a pack could write into `.nc-update` itself.

**Fix:**
- **An exclusive folder per update.** Each update creates its own folder, `.nc-update/<16 random hex>`,
  with `create_dir`, so it fails rather than reuse an existing folder. The folder holds an `owner` marker.
- **Only our folders are removed.** A folder is removed only if it has our marker with the matching id.
  `.nc-update` itself is removed only when it's empty. Anything else in it is never touched.
- **Unusable staging is refused.** If `.nc-update` is a file or a junction, the update is refused and
  `recover` leaves it alone.
- **Packs can't write there.** `nc_core::mrpack::RESERVED_DIRS = [".natives", ".nc-update"]`, in any
  letter case. Trailing dots and spaces were already refused. A path that reaches a reserved folder
  through a Windows 8.3 short name (e.g. `NC-UPD~1`) is refused too, by resolving the real name of the
  first folder on disk.

### Related: an update and a launch of the same instance can't overlap

`recover` must not run while the same instance is being updated, so a launch now holds the instance's
update slot until its game is registered. The update takes the slot first and then checks for a running
game. Whichever order they come in, only one of them proceeds. This also closes one case of the audit's
unconfirmed launch race hypothesis: a launch and an update at the same time.

## Interruption, disk errors and recovery

| Situation | What happens |
|---|---|
| Cancel or download failure before any file moves | Nothing in the instance changed; only our own folder is removed. Tested: `a_cancelled_update_changes_nothing` and the existing broken-SHA-512 integration test. |
| Any rename fails (disk error, locked file, access denied) | Everything done so far is undone. Tested with a failure injected before **each** of the moves and before the commit marker: `a_failure_at_any_step_restores_the_instance`. |
| A file in the way, a link on the way, or a file changed since the plan | Refused in preflight; no journal is written. Tested: `preflight_refuses_*`, `a_junction_in_the_instance_stops_the_update_before_any_change`. |
| Disk full | Old files are moved (renamed on the same volume), so backups need no extra space. Writing the journal or the commit marker can fail; that undoes everything. |
| Crash or kill at any point during the moves | Undone by `recover` before the next launch or update. Tested by simulating a crash after every number of moves: `recover_undoes_an_interrupted_update_at_every_point`. |
| Crash after commit, before or after the database write | Kept if the database already has the new version, undone otherwise: `recover_keeps_a_recorded_update_and_undoes_an_unrecorded_one`. Running it again finds nothing. |
| The database write fails | `rollback` puts the old files back: `rollback_after_success_restores_the_old_files`. |
| The user put a new file at a path the undo needs | It isn't overwritten. The old file stays in `.nc-update`, and the error says where. `recover_never_overwrites_a_file_that_appeared_and_keeps_the_backup`. |
| Foreign content in `.nc-update` | Never removed, also by `recover`. `foreign_content_in_staging_is_never_removed`, `an_update_keeps_existing_content_in_the_staging_folder`. |

## pnpm diagnosis (no bypass applied)

I reproduced the audit's exact command, `pnpm install --offline --frozen-lockfile --ignore-scripts`, on a
copy of the manifests in my scratch folder, using pnpm 12.18.0 (as `packageManager` pins) and Node
24.22.1.
- **It passes here.** pnpm printed "Lockfile passes supply-chain policies (verified 18h ago)", and all
  126 packages came from the local store (0 downloaded).
- **The lockfile is consistent, and neither the repository nor my pnpm config changes any policy.**
  There is no `.npmrc`; `pnpm-workspace.yaml` only lists `apps/*`; `minimumReleaseAge` and
  `trustPolicy` are at pnpm's defaults.

The audit's failure is environmental:
- **`ERR_PNPM_NO_OFFLINE_META`.** `--offline` needs registry metadata already cached on that machine.
  The audit worktree had no `node_modules` and, evidently, a store or cache without metadata for
  `@babel/runtime`.
- **The supply-chain policy failure.** pnpm checks the lockfile against registry metadata, such as
  publish dates for the release-age policy. Offline it can't fetch that metadata, so the check fails
  closed. That's correct, not a sign of a bad dependency.
- **The Corepack message.** pnpm 12.18.0 wasn't in Corepack's cache on that side, so it announced a
  download.

**Approved way to fix it there:** run one normal online install through the registry, with the policies
left on (`pnpm install --frozen-lockfile`). After that, offline installs work.

## Tests run (all on this machine, Windows 11)

| Suite | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (after one test helper got `allow(unwrap_used)`) |
| `cargo test --workspace` | **211 passed**, 0 failed, 4 ignored (network suites); was 186 |
| `pnpm typecheck` / `pnpm test` | clean / **36 passed** (no frontend changes) |
| `e2e/modpack-update.mjs` (real Modrinth, debug build) | **21/21**: cancel changes nothing, the update applies, no staging folder left |
| `e2e/modpacks.mjs` / `pack-activation.mjs` / `packs.mjs` | **25/25** / **24/24** / **26/26** |

New tests (25):
- **`confined::tests` (6):**
  - writes inside the root;
  - a file where a folder is needed;
  - unpredictable temp names;
  - a **real directory junction** refused;
  - a root that is itself a junction is allowed;
  - a folder **swapped for a junction after the check** is caught;
  - file symlinks are refused, or the test is skipped with a message when Windows doesn't allow creating
    them (no Developer Mode). Here it ran.
- **`packupdate::tests` (12, replacing the old apply test):**
  - a failure at every step;
  - a crash at every point;
  - recorded vs. unrecorded;
  - rollback;
  - no overwrite during undo;
  - foreign content;
  - staging that is a file;
  - staging reserved;
  - a junction in the instance;
  - a junction at `.nc-update`.
- **`tests/update_safety.rs` (7, public API):** the audit's three reproductions, now asserting the
  fixed behavior, plus cancellation, packs writing into `.nc-update`, and junctions for overrides,
  updates and staging.
- **`nc-core`:** the existing reserved-path test now also covers `.nc-update`, in every letter case and with a trailing dot or space.

Not run:
- **The audit's own `audit_findings.rs`.** It asserts that the defects reproduce, so it now fails by
  design, and it is written against the old `update` signature. The regression tests above mirror its
  three scenarios.
- **`cargo deny` / `pnpm audit`.** No dependency changed.

## Remaining risks

- **Races by a local process (001).** A process running as the same user can swap a folder for a
  junction between the last check and the rename. Closing this fully needs handle-relative Windows APIs,
  such as `NtCreateFile` relative to a directory handle, which means new unsafe code or a `windows-sys`
  dependency. Such a process can already write anywhere the user can.
- **Recovery needs Night Client (002).** After a crash mid-update, starting the game from another
  launcher before Night Client runs again would see a half-updated folder. Night Client recovers before
  the next launch or update, not at startup. Recovering at startup for every modpack instance is a small
  follow-up if wanted.
- **Not covered in game.** Launching is still blocked by Mojang's approval (R1), so the recovery before
  launch is covered by unit tests, not by a real launch.
- **The audit's other hypotheses are not addressed here:**
  - the redirect policy (`https_only(false)`);
  - `get_bytes` without a size cap;
  - cancellation inside blocking work;
  - the unbounded log channel;
  - the rest of the launch check-then-act race (two launches at once).

  Each is worth its own small change.
- **The operation coordinator the audit recommends** isn't built. The per-instance slot shared by launch
  and update is a small step in that direction.
- **CI hasn't run.** No pull request is open.
