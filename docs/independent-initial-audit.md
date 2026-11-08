# Independent initial engineering review — 2026-11-08

Reviewer: independent reviewer. Snapshot: `a5aea2f` (Stage 15), taken from `feat/stage-15`.
Audit branch: `review/initial-engineering-review`.
Isolated worktree: `C:\Users\User\Desktop\Night Client-audit`.

## Decision and scope

**Do not approve public beta yet.** Three filesystem/recovery defects reproduced below; real Microsoft authentication, entitlement and game-launch evidence remains outstanding. This is an initial risk review, not a complete security certification. No application implementation was changed, no real credentials were used, and nothing was pushed, published or merged.

The immediate task is satisfied by architecture assessment, risk prioritization, a checklist, test-infrastructure recommendations, acceptance criteria and blockers. The added tests are characterization tests: **passing means the defect reproduced**, not that the application is safe. Convert their assertions to the desired behavior when the implementation owner fixes the defects.

## Architecture assessment

The proposed three-crate starting point was appropriate and the implemented boundaries are useful:

- `nc-core` has metadata models, path/rule/argument validation, and no Tokio/reqwest/database dependencies. Keep this boundary.
- `nc-launcher` holds UI-independent I/O services. Public callback and cancellation interfaces, mockable downloads, and fixture-driven metadata tests are a good basis for independent validation.
- `nc-store` provides synchronous SQLite access behind a mutex, WAL, transactional migrations and pre-upgrade backup. `commands::with_db` moves calls to a blocking thread. This differs from the originally proposed single database actor, but does not alone justify a rewrite.
- Tauri commands validate inputs, bridge events and coordinate operations. Capabilities expose explicit commands rather than general filesystem/shell/network access. CSP limits script and connection sources. These are useful controls, not proof that every command is safe.
- React has typed IPC wrappers, a bounded UI log history, virtualized lists and a reduced-motion setting. The IPC types are handwritten rather than generated as proposed; add drift checks before adopting another dependency.

The main architectural weakness is **operation ownership across awaits**. Launch, relocation, content updates and application updates use separate state flags/maps and check-then-act guards. An instance operation needs one reservation spanning validation, filesystem work, process creation and completion/error cleanup. Prefer a small coordinator/guard in the service layer over a broad crate split.

The Tauri modpack command also performs filesystem/DB orchestration itself: files are applied before `set_modpack` and `update_instance`, which are separate store operations. Recovery across these boundaries should have an explicit journal. Hash verification and staging alone do not make a multi-file update transactional.

Avoid splitting the project into nine crates now. First make confined file access, operation ownership and recovery testable and explicit; preserve the working module boundaries.

## Highest-risk components

| Priority | Component | Why it leads the audit queue |
|---|---|---|
| P0 | Filesystem confinement and archive/modpack writes | Hostile metadata reaches writes and deletes; lexical path validation does not handle existing junctions. |
| P0 | Modpack update commit and recovery | Applies multiple files and then database metadata; partial failure and crash consistency matter. |
| P0 | Authentication and launch authorization | Tokens, account switching, entitlement and ownership; real service/game evidence is absent. |
| P1 | Operation coordination and game lifecycle | Concurrent IPC can overlap launch, modification, relocation or update. |
| P1 | Loader processors, Java selection and runtime installation | Executes downloaded or user-selected code; verify trust source, argument construction and cleanup. |
| P1 | Download engine and network trust | Resume/range handling, redirects, bounded response sizes, integrity, retries and cancellation. |
| P1 | Release/update pipeline | Public key/channel configuration, signed artifact/version binding and clean-system installation. |
| P2 | Frontend and performance | Listener lifetime, bounded logs/lists, responsiveness, focus and accessibility at supported DPI. |

## Confirmed findings

### NC-AUDIT-001 — High — Extraction follows an existing junction outside its destination

- **Component/location:** `crates/nc-launcher/src/fsutil.rs:14` (`safe_join`) and `:95` (`extract_zip`), especially parent creation/temp-file creation at `:128–132`. Related override writes: `crates/nc-launcher/src/modrinth/modpack.rs`, `copy_overrides`/`extract_overrides`.
- **Problem:** `safe_join` validates path text and appends segments. It does not verify the filesystem objects traversed. Rejecting symlink entries in an archive does not reject a junction already present in the extraction destination.
- **Reproduction:** create two disposable folders, place a directory junction `root/linked` pointing to the second folder, then extract an ordinary ZIP entry `linked/probe.txt` under `root`. Extraction succeeds and the second folder receives `probe.txt`.
- **Impact:** the promised destination confinement fails when a destination ancestor is a junction. A crafted archive path plus a preexisting junction can overwrite files writable by the current user outside the chosen root. This does not establish a remote attacker can create that junction, or demonstrate privilege escalation.
- **Correction:** implement a shared confined-write abstraction that rejects unexpected reparse points on destination ancestors and temporary files. Use exclusive temporary creation; handle destination changes between validation and opening. Canonicalization alone is insufficient against concurrent replacement. Keep explicit policy for a user-selected root, separate from descendants supplied by metadata.
- **Test:** `crates/nc-launcher/tests/audit_findings.rs:99`, `audit_extract_follows_existing_junction_outside_root`.
- **Verification:** reproduced on Windows 11 using an NTFS junction and only temporary test folders. Archive contains no symlink entry. Broader write paths have not all been reproduced.
- **Fixed acceptance:** reject the operation and leave the external sentinel unchanged; cover directory junctions, file symlinks, temp-file links and concurrent parent replacement.

Windows junctions are implemented using reparse points; see Microsoft's [Hard Links and Junctions](https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions) and [Reparse Point Operations](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-operations).

### NC-AUDIT-002 — High — Failed modpack application leaves a partially updated instance

- **Component/location:** `crates/nc-launcher/src/modrinth/packupdate.rs:278`, `apply`; mutations at `:299–314`; unconditional staging cleanup at `:199`. Database recording follows the service call in `apps/desktop/src-tauri/src/commands/modpacks.rs:599` and `:629–630`.
- **Problem:** all new files are staged and verified, but application renames/deletes files sequentially. A later I/O error returns immediately without undoing earlier changes. Cleanup discards remaining staging, while the command never reaches the metadata update.
- **Reproduction:** track `a.txt` containing `old`; create a regular file named `blocker`; update with ordered overrides `a.txt=new` and `blocker/b.txt=b`. Applying the first succeeds; creating the second destination's parent fails. The update returns an error, but `a.txt` now contains `new` and staging is gone.
- **Impact:** files and recorded pack version diverge after ordinary failures (path conflict, access denial, sharing violation, full disk). A mixed set of mods/configs can become unlaunchable and retry may classify earlier replacements as user modifications.
- **Correction:** preflight conflicts, preserve old files in an owned backup area and journal the commit; roll back errors and recover interrupted commits at startup. Couple DB metadata updates in a transaction and define recovery for filesystem/DB failure ordering. Preserve recovery material until commit is durable. Do not claim cancellation changed nothing if commit already began.
- **Test:** `crates/nc-launcher/tests/audit_findings.rs:38`, `audit_apply_failure_leaves_an_earlier_file_replaced`.
- **Verification:** deterministic reproduction passed, without network or credentials. Abrupt-process termination and DB failures remain follow-up cases.
- **Fixed acceptance:** injected error at every rename/remove/store step restores original files and metadata, or leaves a durable journal that automatically restores/finishes before launch.

### NC-AUDIT-003 — Medium — Update deletes a preexisting `.nc-update` folder without ownership validation

- **Component/location:** `crates/nc-launcher/src/modrinth/packupdate.rs:186` and `:199`; `crates/nc-core/src/mrpack.rs:263`, `instance_path_allowed`.
- **Problem:** update recursively removes the fixed staging folder before doing any work. It does not verify a launcher ownership marker. Pack path validation excludes `.natives`, but allows `.nc-update`, so pack content can also use the same namespace.
- **Reproduction:** create `.nc-update/user.txt` in a disposable instance, then run a successful update with an empty pack/ZIP. The preexisting file disappears.
- **Impact:** untracked user or pack content in this folder is silently deleted. A pack using the staging namespace can also collide with the update's own files. No traversal is needed.
- **Correction:** reserve internal namespaces case-insensitively for index and override paths; allocate a unique owned staging directory and only clean directories created/recognized by this operation. Refuse or preserve unrecognized existing folders. Coordinate this with journal recovery from finding 002.
- **Test:** `crates/nc-launcher/tests/audit_findings.rs:74`, `audit_update_deletes_preexisting_user_staging_folder`.
- **Verification:** deterministic reproduction passed. A complete malicious staging-collision scenario was not exercised.
- **Fixed acceptance:** existing unowned content survives; `.NC-UPDATE/...` is rejected in both index files and overrides; stale owned transactions recover safely.

## Hypotheses and follow-up investigations (not counted as reproduced defects)

| Question | Evidence and next experiment |
|---|---|
| Can two launches start the same instance? | `commands/game.rs:69` checks the map, awaits `prepare_launch` at `:84`, spawns at `:129`, and inserts at `:137`. No reservation is visible across that await. Test concurrent IPC against a fake process backend/barrier; require exactly one spawn. |
| Can launch/content changes overlap pack update or relocation? | Pack updates reserve `modpack_updates`; game launch checks games/queue and relocation flag separately. Enumerate all command pairs with a deterministic barrier, including account switch, delete and application update. |
| Do redirects preserve host/scheme policy? | Downloader validates the initial URL, builds with `https_only(false)` and does not configure a redirect policy in its constructor. Reproduce HTTPS redirects to HTTP/disallowed hosts using local test TLS. Do not assume initial URL validation constrains redirect targets. |
| Can metadata exhaust memory or run indefinitely? | `Downloader::get_bytes` collects `resp.bytes()` without an explicit byte cap; a read timeout does not bound an endlessly streaming response. Test streaming oversized manifests/API responses and impose service-specific caps. |
| Are cancellation and shutdown bounded during extraction/hashing? | Blocking extraction/apply closures lack cancellation arguments; measure worst cases and define the commit boundary. Tokio documents that already-running `spawn_blocking` work cannot be aborted: [task documentation](https://docs.rs/tokio/latest/tokio/task/). |
| Can log production outrun the UI bridge? | `commands/game.rs` uses an unbounded channel, and flushes at 500 lines as well as every 100 ms. Stress sustained large lines/output; record queue growth and event rate before concluding it is bounded. |

## Initial audit checklist

`[x]` means inspected/executed only at the stated scope, not blanket certification.

- [x] Snapshot repository and read proposal/current architecture, security/stage reports and CI/release configuration.
- [x] Check crate dependencies and pure-core boundary; inspect SQLite blocking wrapper/migration design.
- [x] Inspect capabilities/CSP and updater configuration/guards; neither complete command authorization coverage nor deployed signature trust is certified.
- [x] Read PKCE/loopback, token-chain and refresh-token rotation implementation; existing mock account/auth tests pass.
- [ ] Validate Microsoft login/refresh/logout/account switching with owner-controlled accounts and approved application ID; check denial/child/no-entitlement paths using mocks.
- [ ] Audit every filesystem entry point with existing reparse points and link races; expand beyond ZIP extraction.
- [x] Reproduce multi-file apply failure and staging-name deletion; no fixes applied.
- [ ] Prove cancellation, crash recovery and filesystem/DB consistency during install, repair, mods and modpack update.
- [ ] Audit launch arguments/classpath/natives/Java by version fixtures and real Vanilla/Fabric/NeoForge launches; test instance isolation.
- [ ] Review processor integrity/provenance, inherited environment, cancellation and orphan cleanup for NeoForge/Forge.
- [ ] Complete download redirect, truncation/range/size/hash, disk-full and rate-limit failure matrix.
- [x] Run library tests and check their ignored service-dependent coverage.
- [ ] Independently run frontend typecheck, tests, accessibility/keyboard/DPI/listener checks and Windows E2E; dependency restore is currently blocked below.
- [ ] Obtain current dependency-advisory results and a successful hosted CI run for the audited commit.
- [ ] Verify signed update positive/negative/replay cases and clean Windows installer/uninstaller on Win10 and Win11.
- [ ] Independently measure complete startup, process-tree resources, interactions, installs and graceful shutdown.
- [x] Confirm LICENSE, NOTICE, PRIVACY and font-license files exist; dependency-license compliance still needs a current audit result.

## Recommended test infrastructure

Extend the existing Rust tests, `wiremock`, `tempfile`, Vitest/Testing Library, and WebView2 E2E driver; do not replace them.

1. **Every PR:** locked Rust tests, fmt, Clippy, frontend typecheck/tests, production UI build, dependency/license checks. Add deterministic IPC lifecycle tests and security fixtures. Keep no-credential offline fixtures in the required gate.
2. **Filesystem/failure suite:** confined disposable roots, NTFS junction fixtures, exclusive temp-file collisions, case collisions, access-denied/sharing-violation/full-disk injection. Put file operations behind a small injectable interface where deterministic failure placement is otherwise impossible. Use child processes to terminate at commit barriers; verify restart recovery.
3. **Store suite:** retain stepwise migrations, backup, rollback, newer-schema and preservation tests. Add filesystem-update/DB-commit failure tests and combined metadata transactions. Never use a user's actual database.
4. **Minecraft fixture matrix:** 1.8.9, 1.12.2, 1.16.5, 1.20.1, 1.21.1 and a verified 26.x fixture; rules, Java selection, classpath ordering, natives, legacy assets, inheritance and malformed manifests. Loader processor tests use official artifacts in separately marked network runs.
5. **Auth suite:** fake OAuth/XBL/XSTS/Minecraft endpoints; PKCE/state, callback timeout/hostile requests, entitlement/profile denial, rotated-token persistence and concurrent renewal/account switching. Canary tokens must never appear in IPC, diagnostics, logs or panic output. Live credentials stay outside CI and artifacts.
6. **Frontend suite:** event-listener cleanup under mount/unmount, progress/error/cancel states, stale async responses, keyboard/focus and 100/125/150/200% DPI. Use realistic 0/60/600/1500/4000-item lists. Check memory after repeated navigation and large log floods.
7. **Windows release validation:** manual protected release-candidate job, clean Win10/Win11 VMs, real packaged WebView2, installer/update tamper tests. Archive sanitized logs, environment, commit/artifact hashes and test output. Existing manual E2E calls live Modrinth; separate deterministic PR tests from network-dependent scheduled/manual tests. Publish E2E failure artifacts.

## Measurable acceptance criteria

These are proposed gates/targets for the implementation owner; unmeasured is not pass.

| Area | Acceptance evidence |
|---|---|
| Security defects | All three confirmed defects fixed and desired-behavior regression tests pass; no unresolved Critical/High finding before beta. |
| Authentication | Approved release client ID; interactive login, restart/refresh, revoke, account switch and owned/unowned paths verified; zero canary secrets in exported logs/IPC/diagnostics. Live owner login recorded only as sanitized result. |
| Game correctness | Clean Vanilla/Fabric/NeoForge installs reach title screen, enter/save/reopen a world and exit cleanly; repeat launch works without unnecessary download; at least two instances preserve distinct saves/configs/mods. Record MC/loader/Java versions and artifact hash. |
| Install recovery | Truncation, timeout, 404/429/5xx, hash mismatch, sharing violation, disk-full, cancellation and process termination never mark an incomplete install complete; retry repairs to verified contents. |
| Cancellation | Download cancel acknowledgment and network cessation within 1 s under local controlled tests; document and test extraction/hash/commit cancellation boundaries separately. |
| File confinement | Hostile paths, Windows special paths, junctions and temp-file link attacks cannot write/delete outside the authorized root; external sentinels unchanged. |
| Update recovery | Every injected commit failure/restart leaves original state or completes an explicitly journaled commit before launch. Tampered/unsigned installer and version-replay fixtures are refused. |
| CI | Required locked Rust/frontend/lint/security checks pass on the reviewed commit; 0 unexpected skipped deterministic tests. Four presently ignored live suites need separate recorded outcomes. |
| Cold/warm startup | Target p95 <=2 s from OS process creation to a defined usable screen. Report >=10 cold and >=20 warm runs separately; a fresh profile is not an OS-cold run. Track internal `ui_ready` separately. |
| Idle resources | After 30 s settling, sample >=60 s; target total private bytes <150 MiB and average CPU <1% of whole-machine capacity. Report launcher, each WebView2 process class and total; include working set separately. Define units explicitly. |
| Interaction/frames | Target p95 visible feedback <100 ms across >=100 representative actions during idle/download/list scan; at 60 Hz target p95 frame interval <=16.7 ms and p99 <=33.3 ms; report >50 ms tasks and test hardware acceleration both ways. |
| Large lists | 1500 and 4000 entries: bounded DOM count with virtualization, feedback target above, no linear retained-memory growth after 20 open/close cycles. Separate backend hash/list time from render time. |
| Downloads/creation/mods | Controlled local server at concurrency 1/4/8; record throughput, peak memory, host concurrency, error/cancel latency and verified bytes. >=30 instance creations and >=10 fixed mod installations: report p50/p95 latency and exact payload/cache state; establish numeric budgets from this baseline. |
| Shutdown | Target p95 graceful launcher exit <=2 s (>=20 trials), including idle, active transfer and high log volume; no orphan launcher/WebView processes or inconsistent state. Specify and verify intended running-game policy. Forced kill is a separate recovery experiment. |
| Accessibility/installer/legal | Keyboard-only critical flows, focus return and reduced motion pass; clean Win10/11 per-user install/launch/uninstall succeeds; licenses/notices bundled; signing/SmartScreen status accurately documented. |

Performance reference protocol: record Windows build, CPU/RAM/storage, power mode, display/refresh/DPI, WebView2/driver versions, GPU setting, release artifact/commit, profile size, cache state and server/network conditions. Reuse documented reference PC A for continuity and add a four-core/8-GB Windows reference. Preserve machine-readable per-run samples, distribution and failure counts. Do not infer 60 FPS on low-end hardware from high-end 180-Hz frame measurements.

Existing `scripts/bench-startup.ps1` collects process-tree private bytes/working set and CPU, but reads an internal `ui_ready` duration beginning inside `run()` and forcibly stops processes. It does not measure OS-cold startup, usable-screen readiness or graceful shutdown. Existing benchmark tables are implementer-reported historical results, not independently remeasured here. No new performance numbers are claimed.

## Independent verification and limitations

Environment observed: Windows 11 Enterprise 10.0.26300 x64; rustc/cargo 1.99.1; Node 24.22.1. No full reference-hardware performance run was performed.

- `cargo test -p nc-core -p nc-store -p nc-launcher --locked --offline --quiet`: **186 passed, 0 failed, 4 ignored**, including 3 audit characterization tests. The ignored tests require real game/loader/Modrinth services. Mock tests use local servers, so offline dependency mode does not prohibit their loopback traffic.
- `cargo test -p nc-launcher --test audit_findings --locked --offline -- --nocapture`: **3/3 reproduced**.
- `cargo fmt --all -- --check`: passed after formatting the new test file.
- `cargo clippy -p nc-core -p nc-store -p nc-launcher --all-targets --locked --offline -- -D warnings`: passed.
- Frontend dependency restore attempted with `pnpm install --offline --frozen-lockfile --ignore-scripts`: failed with `ERR_PNPM_NO_OFFLINE_META` for `@babel/runtime` and a supply-chain policy failure. Corepack also announced a pnpm bootstrap download before pnpm's offline restore. No policy bypass was applied; frontend tests/typecheck/build were not independently run.
- Full Tauri workspace build/E2E, real login/gameplay, hosted CI, live dependency advisories, installed update channel and clean VM installer were not independently verified.
- No private credentials or token stores were read. Only disposable temporary fixtures were modified by reproduction tests.

## Blockers before implementation/release handoff

1. **Implementation owner:** resolve findings 001–003 with narrow patches and fixed-behavior tests before beta; agree transaction/ownership policy before adding more update features.
2. **Project owner:** confirm approved Microsoft/Mojang client ID and arrange manual end-to-end account/entitlement/game tests. Source reports say this remains blocked; current approval state was not queried and no credentials are requested in chat.
3. **Build/test owner:** restore dependencies through the normal approved registry/policy path, resolve the pnpm policy failure, run frontend/full-workspace/E2E gates and attach current hosted CI/advisory results. The offline restore failure does not establish dependency maliciousness.
4. **Release owner:** provide release-key/channel configuration and clean Win10/11 signed-update/installer evidence; no production change or publication is authorized by this audit.
5. **QA owner:** capture independent performance baselines using the protocol above. Existing internal startup numbers are insufficient evidence for the full startup target.

Do not carry forward stale claims without checking: the Stage 7 security document still says no updater exists, whereas Stage 8 and current code do implement one. Likewise the old report's repository-placeholder issue is already addressed in `nc_launcher::user_agent`; remote repository existence and CI history were not checked in this pass.
