# Independent acceptance review — RR-001 to RR-004

**Verdict: APPROVED for the scoped fixes in `3805ce4`.** This approves the reviewed corrections, not public-beta release readiness, publication or merging. No new blocking finding was confirmed in this pass.

Date: 2026-11-09 (Europe/Warsaw). Reviewed `231950d..3805ce4`: implementation commits `1a89cf9`, `bb1f4fd`, documentation commit `3805ce4`. Independent worktree: `C:\Users\User\Desktop\Night Client-review2`.

The implementer's `docs/audit-rereview-fixes-report.md` was treated as a statement of changes and claimed results, not verification evidence. The previous reports and their acceptance criteria remain the review baseline. The other worktrees were preserved; no application implementation was edited, and nothing was pushed, merged or published.

## Finding disposition

| Item | Acceptance | Independently assessed evidence |
|---|---|---|
| RR-001 — atomic metadata commit | **Met for the reproduced defect. Closed.** | `nc-store/src/modpacks.rs:47`, `record_modpack_update`, writes pack, tracked files, instance patch and acknowledgment inside one SQL transaction. Store tests inject a failing instance trigger and invalid file row and verify rollback of every record. The command uses this API at `commands/modpacks.rs:708` and queries the acknowledgment at `:718` before deciding to undo an uncertain result. |
| RR-002 — cleanup through swapped parent | **Met for the deterministic parent-swap defect. Closed.** | `nc-launcher/src/confined.rs:199` checks the source before persist; `remove_temp` at `:273` rechecks ancestry and confinement before removal. Extraction/override/download failure cleanup calls the checked helper. The original outside sentinel survives in the regression test. This does not close the separately documented final check/use race. |
| RR-003 — edits after planning | **Met for the reproduced defect. Closed.** | Steps carry expected old SHA-1. After moving an old file into backup, `packupdate.rs:633` verifies its contents and rolls back on mismatch. Tests cover the progress-callback edit, enabled and disabled replacement, removal, and an edit immediately before the move. Destination appearance is refused by no-replacement rename. |
| RR-004 — same-version acknowledgment ambiguity | **Met. Closed.** | A 128-bit update ID names each transaction and appears in journal format 2. `packupdate.rs:745` validates journal format/ID, and recovery at `:816` compares the exact database acknowledgment to that journal's ID. Same-target pack versions are no longer used as evidence of commit. Tests verify own-ID, other-ID and absent-ID cases. |
| Q5 — launch snapshot before reservation | **Addressed by code inspection.** | `commands/game.rs:56` takes the instance reservation before reading its metadata, and releases it after process registration. This closes the previously described update-between-read-and-reservation interleaving. No authenticated concurrent-launch IPC test was run. |

Paths in the table are relative to `crates/` for Rust services/store and to `apps/desktop/src-tauri/src/` for commands.

The SQL grouping now has a real transaction boundary, consistent with SQLite's [transaction semantics](https://www.sqlite.org/lang_transaction.html). The Windows rename primitive uses `MoveFileExW` with flags 0, excluding replacement and cross-volume copy flags; Windows tests exercise destination conflicts and long local paths. See Microsoft's [MoveFileExW documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).

## Additional independent integration evidence

Added `crates/nc-launcher/tests/review2_acceptance.rs`, containing two cross-layer acceptance tests using disposable files and a real SQLite store:

1. **Successful record followed by interrupted cleanup:** apply a filesystem update, commit its metadata and exact ID through `Store::record_modpack_update`, drop `Pending` without calling `finish`, reopen the database and recover using the stored ID. Recovery retains the new file, pack version, file hash and Minecraft version; cleans staging; and is idempotent.
2. **Failed record followed by interrupted rollback:** apply the files, then cause instance validation to fail inside the metadata transaction. Verify that the prior acknowledgment survives. Drop `Pending` before rollback, reopen the database and recover using that prior ID. Recovery restores the old file, metadata and hash, and cleans staging.

Both passed. The second test injects a validation error inside the SQL transaction; it is not a simulated storage-device failure. The implementer's separately rerun store test uses a real SQLite trigger failure and supplies the SQL-error coverage. These tests compose the actual filesystem and store APIs; they do not exercise Tauri IPC or real authentication.

## Checks actually run

Environment: Windows 11 Enterprise 10.0.26300 x64, Rust/cargo 1.99.1, Node 24.22.1, pnpm 12.18.0. Tests/builds used only the new isolated worktree. Dependency policies were not disabled.

| Check | Independent result |
|---|---|
| `pnpm install --frozen-lockfile` | Passed; policy check passed, dependencies reused from the local store. |
| `pnpm typecheck` | Passed. |
| `pnpm test` | **36 passed**, 3 files. |
| `pnpm --filter @night-client/desktop build` | Passed. Existing >500 kB chunk warning remains informational for this review. |
| `cargo test --workspace --locked --offline --quiet` | **232 passed, 0 failed, 4 ignored.** Baseline fix tests, excluding the two subsequently added reviewer tests. |
| `cargo test --workspace --test review2_acceptance --locked --offline -- --nocapture` | **2 passed**, 0 failed. |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed with warnings denied, including the independent acceptance-test target. |

The full workspace run includes all 8 `update_record` tests (including v8-to-v9 migration preservation), all 4 `rereview_regressions` tests, all 7 `update_safety` tests, and `a_real_crash_at_any_point_is_recovered`.

I inspected the abort-test implementation: it launches a child test executable, aborts without unwinding at journal/move/commit boundaries, then recovers and compares the complete non-staging fixture tree. It tests acknowledged and unacknowledged cases and a second recovery. This provides process-crash evidence that the first review lacked. It does not provide storage-power-loss evidence. The test's `Point::Journal` hook is before journal writing; `Move(0)` exercises the later journal-written state.

`--offline` constrains dependency resolution, not loopback mock-server traffic. Four ignored tests require real game/loader/Modrinth downloads. The passing totals do not imply those scenarios were exercised. File-symlink test code can return early when privileges are unavailable; this run's total alone is not proof of that optional fixture. Directory-junction regressions and the process-abort parent test did execute.

## Scope and remaining limitations

The acceptance above is deliberately tied to the confirmed RR reproductions. It does not turn broader guarantees into proven properties:

- **Same-user races remain.** `Confined` validates pathname state and then operates. It is not a handle-relative sandbox. A concurrent same-user process can still change ancestry between the final check and a filesystem operation. RR-002's deterministic cleanup escape is fixed; the full concurrent-replacement requirement from NC-AUDIT-001 remains a separate hardening item. Do not advertise unconditional path confinement.
- **Open handles can write after hashing.** Post-plan edits that exist by the backup-hash check are detected. A writer retaining an open handle can change the moved backup after that check; its content may subsequently be discarded. This documented limitation is outside the reproduced progress-callback case.
- **Process crashes are covered; power loss is not guaranteed.** No directory-entry durability or real power-cut test was added. Narrow any wording that still implies a universal power-loss guarantee, including broad crash-recovery comments. The implementation report now explicitly discloses this limit.
- **Not every filesystem path is confined.** Non-modpack downloads use `root: None`; settings and NeoForge outputs still have their earlier path policies. They were not certified safe by this scoped approval.
- **Historical unreleased journals:** ownership now recognizes 32-character transaction IDs. Old 16-character directories from the prior unreleased format are not recognized as current transactions. No migration/recovery test for a half-applied old-format development update was run; preserve and handle such data explicitly if upgrading an affected development profile.
- **8.3 aliases and non-Windows paths:** no live short-name-volume or non-Windows/remote-share test was performed. Windows x64 local disposable folders were the verified platform.
- **Exceptional command branches:** acknowledgment-aware handling is improved, but actual Tauri worker panic, database-unreadable-after-commit and authenticated reservation interleavings were not independently fault-injected. In particular, the acknowledged-error fallback still uses a fallible `get_instance` read; avoid presenting every possible generic error as proof that nothing changed. This is a residual untested error-path concern, not a newly reproduced blocker.

## Original audit status and release handoff

- **NC-AUDIT-001:** the original static-junction extraction defect and the later deterministic cleanup escape are corrected in exercised cases. Complete race-resistant confinement across all write paths remains unverified/unimplemented as noted above.
- **NC-AUDIT-002:** the reproduced partial-apply and split-metadata defects are corrected; process-crash recovery and exact transaction acknowledgment now have independent evidence. Power-loss and arbitrary external-writer guarantees remain limited.
- **NC-AUDIT-003:** the original staging ownership/deletion criteria remain satisfied by the rerun tests. Foreign content survives and reserved staging is refused for pack input.

No current live Microsoft login/entitlement/gameplay, real-Modrinth desktop E2E, signed-update/installer, clean Win10/11 VM, hosted CI, advisory audit or performance measurement was performed in this pass. The earlier beta-readiness checklist remains open where independent evidence is absent. This scoped approval does not authorize release, push, publication or merging.

The implementer can carry these fixes forward. Retain the regression tests and document the residual threat/durability model; pursue the broader release checklist separately.
