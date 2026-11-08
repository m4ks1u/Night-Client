# Re-review fixes report: RR-001 to RR-004

Branch: `fix/audit-001-003` (local, new commits on top of `231950d`, not pushed or merged) · Date:
2026-11-09 · Re-review: independent reviewer, `review/audit-fixes-rereview` @ `5a6abe7`,
`docs/independent-audit-rereview.md` (verdict REVISE)

I fixed the four findings and the launch-snapshot question (Q5), and turned each of the independent review's
characterization tests into a regression test. **Nothing has been pushed, merged or published, and
the independent review's worktrees were not modified** (they were only read). No new crates or dependencies. There is one
database migration (v9).

## Findings: cause and fix

### RR-001 (High): pack metadata was committed before instance metadata

**Cause (confirmed).** The update command called `set_modpack` (its own transaction) and then
`update_instance` (a separate write). If the second write failed, the database held the new pack and
files but the old Minecraft and loader versions. The command then rolled back the files, which left the
records describing files that were no longer there.

**Fix.**
- **One transaction.** `Database::record_modpack_update(instance, pack, files, patch, update_id)` writes
  the pack row, its file rows, the instance's versions and the update's id in **one** SQLite
  transaction. Any failure records nothing.
- **Shared helpers.** `set_modpack` and `update_instance` share the same internal helpers
  (`write_modpack`, `patch_instance`); `update_instance` is now transactional too.
- **Migration v9.** It adds `instance_modpack.last_update_id`, which is NULL or 1–64 characters. A new
  import clears it.
- **Unknown outcome.** If the record call returns an error, the command reads the id back:
  - **The id is recorded:** the update stands (an error after the commit).
  - **It is not:** the files are rolled back.
  - **The database can't answer:** the files and journal are left for recovery, and the error is
    `update_recovery`.

### RR-002 (Medium): cleanup after a failure could delete outside the root

**Cause (confirmed).** On error, `persist`, `write` and the download, override and extraction paths
removed their temporary file with a plain `remove_file` on the joined path. If the parent folder had been
swapped for a junction, that removal followed it.

**Fix.**
- **Checked cleanup.** All failure cleanup goes through `Confined::remove_temp`. It deletes the file only
  while its folder still passes the confinement check. Otherwise it leaves the file and logs a warning.
  `persist` also checks the temporary path before using it.
- **Confined downloads.** Modpack downloads now give the download engine the root
  (`DownloadTask::root: Some(Confined)`). The engine creates the destination folders through it and
  checks the `.part` file and the destination before opening, removing and renaming them. Other
  downloads (game files, loaders, Java) pass `None` and behave as before.

### RR-003 (Medium): files modified after planning were overwritten

**Cause (confirmed).** The plan hashed files once. Preflight compared against the plan, not the disk at
move time, so an edit during the download (the independent review's progress-callback case) was moved aside and dropped.
`rename` also replaced an existing destination.

**Fix.**
- **Hash checked at move time.** Every Replace and Remove step carries the SHA-1 the plan saw. Right
  after the old file is moved aside, its hash is checked. A mismatch fails the step ("was changed since
  the update was planned, so nothing was replaced"), and the undo puts everything back, including the
  edited file.
- **No overwriting renames.** Every confined rename refuses to replace an existing file:
  - Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`, using a `\\?\` path so long paths work.
  - Elsewhere: hard link, then unlink.

  So a file that appears at a destination just before its move is never overwritten.

### RR-004 (Medium): recovery could not tell two updates to the same version apart

**Cause (confirmed).** `recover` kept a committed update when the database's pack version equalled the
journal's target. A second update to the same version (a repair, or a retry after a crash) matched
before anything was recorded.

**Fix.**
- **Update id.** Each update gets an id: 128 random bits as 32 lowercase hex digits, which is also its
  folder name.
- **Journal format 2.** The journal records `format` and `id`. A journal with another format, or whose
  id doesn't match its folder, is kept and reported, never trusted.
- **Acknowledged by id only.** `recover(game_dir, acknowledged)` keeps an update only if it is committed
  **and** `acknowledged` is its own id. The command passes the database's `last_update_id`. The pack
  version plays no part.

### Q5: a launch read the instance before taking the slot

**Fix.**
- **Slot first.** `launch_instance` takes the instance's update slot (`reserve`) before it reads the
  instance from the database. Recovery still runs before `prepare_launch`.
- **Released at registration.** The slot is released once the game is registered. So no update can
  change the versions between the read and the launch.

## Failures between individual filesystem operations

Ordinary rollback tests weren't treated as proof of crash consistency.

**Real crash test.** `packupdate::tests::a_real_crash_at_any_point_is_recovered` re-runs the test binary
as a child process. The child calls `std::process::abort()` at a chosen point:
- after the journal is written;
- after each of the six moves;
- before the commit marker;
- after the commit marker;
- after a full apply.

That is 10 points. Each runs twice, with the update acknowledged or not.

**What the parent checks.**
- The child really aborted (not exit code 101, not a panic).
- `recover` leaves the instance exactly as before (when not acknowledged) or exactly as after (when
  acknowledged and committed).
- No `.nc-update` folder of ours is left behind.

In-process fault injection (`apply_with` with a `Point` hook) covers the same points without a crash.

**Change-detection tests.**
- `an_edit_just_before_a_move_is_caught_and_undone`: an edit right before a file's move.
- `a_file_appearing_just_before_its_move_is_not_overwritten`: a file appearing right before its move.

## Regression tests (the independent review's characterization tests, reversed)

| independent reviewer test | Now asserts | Where |
|---|---|---|
| `rereview_edit_after_planning_is_overwritten` | the update fails, the edit is kept, no staging left | `nc-launcher/tests/rereview_regressions.rs` |
| `rereview_same_target_recovery_accepts_unrecorded_update` | recovery with no id or another id undoes; its own id keeps it | same file (2 tests) |
| `rereview_failed_persist_deletes_external_temp_after_parent_swap` | the outside sentinel survives, nothing is written outside | same file (real junction) |
| `nc-store/tests/rereview_transactions.rs` (split commit) | a failing instance write (trigger) records nothing; two updates to one version get different ids | `nc-store/tests/update_record.rs` (8 tests) |

NC-AUDIT-001/003 regressions still pass unchanged (`tests/update_safety.rs`, 7 tests). Their only edit
was the removed `target` argument.

## Test results (run on 2026-11-09, final tree)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | **232 passed, 0 failed, 4 ignored** (round 1: 211) |
| `pnpm typecheck` | clean |
| `pnpm test` | 3 files, **36 passed** |
| `cargo build -p night-client` | ok |
| e2e `modpack-update.mjs` (real Modrinth) | 21 PASS, 0 FAIL |
| e2e `modpacks.mjs` | 25 PASS, 0 FAIL |
| e2e `pack-activation.mjs` | 24 PASS, 0 FAIL |
| e2e `packs.mjs` | 26 PASS, 0 FAIL |

Per binary (non-empty):
- **Libraries:** nc-core lib 36, fixtures 12; nc-launcher lib 89 (packupdate 22, confined 8).
- **nc-launcher integration tests:** accounts 6, auth 12, download 15, loader_versions 4, modpack 3,
  modrinth 5, packupdate 1, rereview_regressions 4, update_safety 7.
- **nc-store:** lib 1, store 26, update_record 8.
- **App:** lib 3.

The 4 ignored tests download real game files, real loaders or real Modrinth packs, and are run manually
(`loaders`, `matrix`, `modpack`, `modrinth`). They were not run for this report.

## Remaining limitations

- **Same-user races.** `Confined` checks paths, then acts on them. A process running as the same user
  can still swap a folder for a junction between a check and the operation. Closing that fully needs
  handle-relative Windows APIs (`NtCreateFile` with a root handle). Such a process can already write
  anywhere the user can.
- **Open handles after the hash.** A file is hashed after it is moved aside. A program that keeps it
  open and writes through that handle after the hash writes into the moved-aside copy, which is dropped
  once the update is recorded.
- **No power-loss guarantee.** File contents and the journal are flushed, but directory entries are not
  (`FlushFileBuffers` on a directory isn't used). Crash consistency is tested by process abort, not by
  power loss.
- **Not everything is confined.** Game-settings writes (`options.txt`, `iris.properties`) and NeoForge
  installer outputs still rely on `safe_join` plus atomic writes, not `Confined`.
- **8.3 short names.** The `.nc-update` short-name rule is implemented, but there is no test with real
  8.3 names, which are off on many volumes.
- **Recovery needs the launcher.** Starting the game outside Night Client before the next launch or update
  from Night Client sees an interrupted update as it was left.
- **Out of scope.** The other hypotheses in the independent review's re-review that were not confirmed findings were not
  addressed here.
