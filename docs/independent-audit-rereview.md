# Independent re-review: NC-AUDIT-001–003

**Verdict: REVISE.** Reviewed fix snapshot `231950d` against `a5aea2f` on 2026-11-08 (Europe/Warsaw). The original reproduction cases now pass as regression tests, and the transaction design is a useful improvement, but the complete acceptance criteria are not met.

This review treats the implementation review request as the implementer's scope and questions, and `docs/audit-fixes-report.md` as claims to verify. Results below are independent. The original audit worktree and implementer's tracked files were left unchanged. Review-only test files and this report live in `C:\Users\User\Desktop\Night Client-rereview`. No application patch, push, merge or publication was performed.

## Original finding acceptance

| Original finding | Status | Evidence |
|---|---|---|
| NC-AUDIT-001, filesystem confinement | **Partially met; remains open.** | Static descendant junctions are refused by extraction/overrides/update regression tests. Exclusive random temp creation improves safety. Error cleanup can still delete outside the root after a parent swap (RR-002); download writes still use path prechecks rather than the confined write/rename primitive. Full race resistance is explicitly not implemented. |
| NC-AUDIT-002, transactional update/recovery | **Not met.** | Original blocked-parent scenario is now refused before mutation; rename fault/crash-prefix tests and rollback pass. Pack metadata and instance launch metadata are still separately committed (RR-001). Preflight does not recheck content (RR-003). Recovery uses target-version equality rather than a transaction acknowledgment (RR-004). |
| NC-AUDIT-003, staging ownership | **Met for the original scope and exercised cases.** | Existing foreign files/folders survive; staging files/junctions are refused; random transaction folders require a matching owner marker; reserved names are checked case-insensitively. The 8.3-alias path exists in code but was not independently exercised on a volume with short-name creation enabled. Ownership markers are ordinary files, not authentication against hostile same-user processes. |

## Findings

### RR-001 — High — Pack and instance metadata do not commit atomically

**Location:** `apps/desktop/src-tauri/src/commands/modpacks.rs:694–696`; `crates/nc-store/src/modpacks.rs:30` (`set_modpack`) and `crates/nc-store/src/instances.rs:222` (`update_instance`, see function definition in the reviewed source).

`with_db` moves work onto a blocking thread; it is not a database transaction. `set_modpack` opens and commits its own SQL transaction, then `update_instance` performs separate writes. If the latter fails, the command rolls files back at `modpacks.rs:710` but leaves the newly committed pack version and tracked hashes in the database. If the process instead dies between these calls, recovery sees the new pack target and the committed filesystem marker and discards the old-file backups, although Minecraft/loader launch metadata still describes the previous instance.

**Reproduction:** create a disposable store with a v1 pack and Minecraft 1.21.1. Add a SQLite trigger that fails updates to `instances`. Execute the exact two-call sequence with a v2 pack record and a valid 1.21.2 instance patch. The second call fails; `get_modpack` returns v2 while `get_instance` remains 1.21.1. This uses a synthetic database failure, not invalid input. The command's subsequent filesystem rollback and crash behavior were inspected in code, not exercised through Tauri IPC.

**Impact:** an ordinary partial database failure or process interruption can produce files, tracked hashes, recorded pack version and launch configuration that disagree. A reported failed/cancelled update may still have changed persistent metadata.

**Suggested fix:** one store API/SQL transaction must commit the pack record, tracked files, Minecraft/loader metadata and a unique update transaction identifier together. On DB failure retain/restore the old DB state before rolling back files. Recovery must use that transaction identifier. Add fault/kill tests before and after each persistent boundary, including the two former store calls.

**Test/status:** `crates/nc-store/tests/rereview_transactions.rs:5`, `rereview_pack_metadata_commits_before_instance_patch_failure` — reproduced. Test passes when the defect is present.

SQLite's [transaction documentation](https://www.sqlite.org/lang_transaction.html) describes explicit transaction boundaries and automatic commits; multiple Rust calls in a blocking closure do not establish a shared transaction.

### RR-002 — Medium — Denied persist deletes a file outside the confined root

**Location:** `crates/nc-launcher/src/confined.rs:206–207`, cleanup in `Confined::persist`.

When destination validation fails, cleanup unconditionally calls `remove_file(tmp)` using the original path. If the temp file's parent was replaced by a junction, this path now names a different file outside the root. The validation correctly rejects the persist, then cleanup follows the junction to delete the external file. The implementer's existing parent-swap test creates this external temp-name file but does not assert it survives.

**Reproduction:** `temp_for("cfg/x.txt")`; move `cfg` to `real-cfg`; replace `cfg` with a junction to a separate disposable folder; put an external sentinel at the same temporary basename. `persist` returns an error and the external sentinel is deleted. This is a deterministic swap between API operations, not a narrowly timed win between the final check and rename.

**Impact:** rejection of an unsafe write still mutates an external directory. Exploitation requires the parent to be changed and an external file at the temporary name; this is not demonstrated as a remote or privilege-escalation attack. It violates the original external-sentinel acceptance criterion and is avoidable independently of the documented final rename race.

**Suggested fix:** do not remove a temporary pathname whose parent/identity is no longer trusted. Retain/leak the original temp file when safe cleanup cannot be established, or use a retained parent/file handle for deletion. Apply this rule to all error cleanup paths, including copy/write/download cleanup, and assert every external sentinel survives the swap tests.

**Test/status:** `crates/nc-launcher/tests/rereview_findings.rs:102`, `rereview_failed_persist_deletes_external_temp_after_parent_swap` — reproduced on a real Windows junction. Test passes when the deletion reproduces.

### RR-003 — Medium — Preflight checks file type, not content changed since planning

**Location:** `crates/nc-launcher/src/modrinth/packupdate.rs:527–541`, replacement/removal checks in `prepare`.

For replacement/removal the new preflight only checks that the destination is still a file. The old content hash observed by `plan` is not carried into the step/journal or rechecked. A user or another allowed writer can edit a config while downloads run; the update then replaces it, and `finish` deletes the backup containing that edit. The report's claim that files changed since planning are refused is not established by the added test: that test covers an Add destination that newly appears, not a modified existing file.

**Reproduction:** start with tracked `a.txt=old` and an override `a.txt=new`. In the public update's final download-progress callback (after planning and before preflight), write `a.txt=user edit during download`. The update succeeds; after `finish`, only `new` remains.

**Impact:** edits during the planning/download window are silently lost despite the preservation policy. This is an incomplete preservation/preflight guarantee, not a claim of a newly introduced remote vulnerability.

**Suggested fix:** store the expected on-disk content identity with Replace/Remove steps and revalidate immediately before backup/commit. Refuse/replan on mismatch. Keep the ordinary application operation reservation in place too; another process or editor is still possible. Add changed-content tests for enabled/disabled replacement and removal, not only existence conflicts.

**Test/status:** `crates/nc-launcher/tests/rereview_findings.rs:41`, `rereview_edit_after_planning_is_overwritten` — reproduced without network. Test passes when the edit is overwritten.

### RR-004 — Medium — A target version cannot identify which update the database acknowledged

**Location:** `crates/nc-launcher/src/modrinth/packupdate.rs:737` (`committed && recorded == Some(j.target.as_str())`).

Recovery interprets equality of the current database version and the journal target as proof this particular filesystem transaction was recorded. If that target was already recorded before the operation, the same comparison is true even when no metadata write occurred. The public API and command do not reject same-target operations. The test demonstrates that ambiguity with a same-target transaction that changes a file and drops `Pending` before any DB acknowledgment; recovery cleans it as recorded and destroys the old backup.

**Impact/qualification:** version equality alone does not prove the file-list/instance-metadata transaction committed. With an immutable Modrinth archive and an already consistent same-version install, a repeated update may be a no-op, so this test does not show a normal immutable Modrinth version changing its published bytes. It does establish that the recovery protocol cannot distinguish the two states, and RR-001 makes stale/partially recorded states reachable.

**Suggested fix:** journal a random transaction ID and persist that exact ID atomically with all affected metadata. Compare that acknowledgment on recovery; alternatively, explicitly reject same-target operations if they cannot be supported, while still fixing RR-001.

**Test/status:** `crates/nc-launcher/tests/rereview_findings.rs:71`, `rereview_same_target_recovery_accepts_unrecorded_update` — reproduced through the service API. Test passes when the unacknowledged transaction is kept. No live Modrinth same-version scenario was run.

## Answers to the implementer's questions

1. **Junction detection:** observed Windows junction fixtures are detected and refused. `symlink_metadata` is the appropriate non-following metadata query; see Rust's [FileType documentation](https://doc.rust-lang.org/stable/std/fs/struct.FileType.html). This is not certification of every reparse tag/provider. Not every metadata-driven write uses `Confined`: `download_confined` prechecks and then hands paths to the existing downloader; the NeoForge extraction/runtime/download paths still include direct filesystem operations. Those unchanged paths are scope/coverage limitations, not additional newly introduced findings here. `gameopts::write` uses random exclusive `atomic_write` temps but does not confine the `config` ancestry relative to the game root.
2. **Rename race:** document it as a remaining same-user race and avoid an unconditional confinement guarantee. Whether to accept it for beta is a threat-model decision; do not represent the original concurrent-replacement acceptance test as fully passed. RR-002 is an additional avoidable cleanup error and should be fixed even if the final rename race is accepted. Full resistance requires handle-relative/identity-preserving operations rather than another canonicalize check.
3. **Undo rule:** the tested rename prefixes and reverse undo order work for the closed-world fixture with no new external changes. Fault-at-each-move and crash-prefix tests pass. This is not a complete power-loss or arbitrary-writer proof: existence alone does not identify contents, and no real process-kill/storage-power test was run. RR-001/003/004 remain the important commit/recovery gaps found here.
4. **Recovery decision:** after a genuinely atomic DB commit, finishing a committed filesystem transaction is appropriate. The current DB writes are not atomic as a group, and target equality is ambiguous; see RR-001/004.
5. **Launch/update exclusion:** the shared reservation blocks simultaneous launch preparation/recovery and pack-update mutation once it is held. However, launch reads `instance` and computes its install key before acquiring the slot (`commands/game.rs:53–84`, reservation at `:86`). An update can finish while `active_session` awaits, after that snapshot but before reservation; launch then uses an old MC/loader key with new pack files. Acquire the slot before reading instance launch state, or reread/recompute after acquisition. This interleaving is a **code-inspection concern, not an independently executed IPC reproduction**. Do not count the separate preexisting two-launch race as fixed by this review.
6. **Messages:** the cancellation message is not universally accurate: RR-001 can leave new DB metadata after filesystem rollback and still return a non-`update_recovery` error. Also the rollback worker's join failure is mapped to `internal`, and errors before/after `undo` can remain generic I/O. Check confirmed state restoration before emitting “Nothing was changed”; retain a recovery-specific status whenever that is unknown. These error branches were inspected, not all fault-injected.
7. **pnpm:** a normal `pnpm install --frozen-lockfile` passed with all policies enabled (223 entries verified; pnpm 12.18.0). Typecheck, all 36 frontend tests and production build passed. This clears the earlier verification blocker and supports the environment/cache diagnosis; it does not reconstruct exactly which missing metadata caused every earlier policy message.

## What was inspected and run

Inspected the 16-file change list, the relevant implementation/test diffs, the complete `confined` implementation, transaction/recovery implementation and tests, public regression tests, relevant store methods, and Tauri launch/update/reservation coupling. Compared behavior with the initial audit's acceptance criteria, rather than accepting the implementer's report as proof.

Independent environment: Windows 11 Enterprise 10.0.26300 x64, Rust/cargo 1.99.1, Node 24.22.1, pnpm 12.18.0. Source snapshot was detached at `231950d` during testing; no original audit tests were copied onto the fix source.

| Check | Independent result |
|---|---|
| `pnpm install --frozen-lockfile` | Passed with default policies left on. |
| `pnpm typecheck` | Passed. |
| `pnpm test` | 36 passed, 3 test files. |
| `pnpm --filter @night-client/desktop build` | Passed; Vite warned about a >500 kB chunk. No frontend source changed. |
| `cargo test --workspace --locked --offline --quiet` | 211 passed, 0 failed, 4 ignored. Includes all 7 `update_safety` tests and 3 Tauri library tests. |
| `cargo fmt --all -- --check` | Passed. |
| Independent characterization tests | 4 passed: 3 launcher tests and 1 store test. Passing means remaining defect/ambiguity reproduced, not fixed behavior. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed with warnings denied, including the new independent reviewer test files. |

Library mock tests use loopback servers; `--offline` constrains dependency resolution, not fixture-server traffic. New test files are review evidence only and do not change the implementer's test expectations. Commands and exact fixture logic are in those files; rerun using:

```powershell
cargo test -p nc-launcher --test rereview_findings -p nc-store --test rereview_transactions --locked --offline -- --nocapture
```

## Limitations and handoff

- No real Microsoft credentials, token vaults, accounts or game saves were used. Login/gameplay remains unverified.
- Real-Modrinth E2E scripts, ignored live loader/game tests, signed release installation/update flow and clean-system installer were not rerun. Passing unit/integration suites do not certify those outcomes.
- No live advisory audit or performance benchmark was performed; no dependencies changed on this fix branch.
- File-symlink test code can return early when Windows denies symlink creation. I separately ran `cargo test -p nc-launcher --lib file_symlinks_are_refused_when_they_can_be_made --locked --offline -- --nocapture`: it passed with no skip message, so the fixture executed here. Directory junction tests also executed, including independent RR-002.
- Simulated crash-prefix tests are not real power-cut durability tests. Same-user link replacement during the final filesystem call remains a documented limitation.
- Redirect policy, metadata size caps, cancellation inside blocking work, unbounded logging and the broader operation coordinator remain outside this branch's scope; no new finding is raised for them here.

Before approval, fix RR-001–003 and establish an unambiguous transaction acknowledgment for RR-004. Add desired-behavior regression tests for these reproductions and the launch-snapshot concern, then rerun the required gates. Preserve the distinction between an operation successfully undone, an operation committed, and one needing recovery in both state and messages.
