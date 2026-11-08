# Stage 1 report: application foundation

Branch: `feat/stage-1-foundation` (local, branched from `feat/stage-0-poc`, not pushed) · Date: 2026-11-08

## Summary

Night Client is now a real application shell instead of a single proof-of-concept screen:

- **Local database (`nc-store`):** SQLite in WAL mode with forward-only, transactional migrations in
  `PRAGMA user_version`. The database is backed up automatically before an existing file is migrated,
  and a database created by a newer version is refused without being touched. It holds instances and
  settings. **No secrets are stored.**
- **Instances:** create, rename, change memory, open folder, verify files, delete. Each instance has its
  own game folder. The folder name is fixed at creation, so renames never move files. Delete moves the
  folder to `<data>\trash` rather than erasing it. Play time and last-played are recorded on game exit.
- **Screens:** Home (continue playing / first-run), Instances (search, list, status), New instance,
  Instance details (overview, game log), Account, Settings, Diagnostics, with loading, empty, error,
  offline-cache and database-unavailable states.
- **Logging:** rotating daily file in `%LOCALAPPDATA%\NightClient\logs` (7 kept), written through a
  non-blocking writer with a regex redaction pass for JWTs, bearer tokens, OAuth parameters,
  `--accessToken` and long opaque blobs. The Diagnostics screen shows the redacted tail.
- **IPC hardening:** a Tauri app permission manifest. Each of the 26 commands now needs an explicit
  `allow-*` permission, granted to the main window only. Instance IDs, version IDs, settings and memory
  values are validated in Rust.
- **Memory guidance:** total RAM is read through `GlobalMemoryStatusEx`. The recommendation is a quarter
  of RAM clamped to 2–8 GB, the slider tops out at RAM − 2 GB (max 32 GB), and a warning appears above
  2× the recommendation.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **87 passed**, 0 failed (nc-core 24, nc-launcher 46, nc-store 14, app 3) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `pnpm typecheck` / `pnpm test` | clean / **21 passed** |
| Desktop smoke test (debug build, WebView2 DevTools, 19 checks) | **19/19 pass**: create → rename → memory override persisted → log tab → sign-in gate → settings persisted → diagnostics → backend rejects `../` IDs over raw IPC → delete to trash → no uncaught UI errors |
| Startup benchmark (release, 5 runs) | UI ready **291–306 ms**, private memory **176–178 MB**, idle CPU **0–0.12 %** |

New tests cover migrations (stepwise upgrade with data kept, rollback of a failing migration, refusal
of a newer schema, backup before migrating), instance CRUD, folder-name collisions (case-insensitive and
on-disk), settings defaults and corruption fallback, log redaction (positive and negative cases),
trash moves with traversal attempts, memory recommendations, and the React stores and router.

## Not done or not verified

- **CI has still never run.** There's no GitHub remote yet.
- **Sign-in and launch are still not tested end to end** (Mojang app-ID approval, R1). The Play flow was
  verified up to the sign-in gate. Install-then-play and play-time recording run only after a real launch.
- **Memory target still missed** (≈177 MB vs < 150 MB). Unchanged by Stage 1.
- **Hide-while-playing** is implemented but unverified, because it needs a real launch.
- The Downloads, Installed mods and Discover mods screens are **not shown yet**. They arrive with their
  features (Stages 4–5) rather than as placeholders.
- No migration from the Stage 0 `instances\default` folder. There were no users, so it was skipped.

## Remaining risks

| Risk | Note |
|---|---|
| R1 Mojang app-ID approval | Still blocks real sign-in and launch. |
| Game data in roaming `%APPDATA%` | Matches the Minecraft ecosystem, but roaming profiles in domain environments may sync gigabytes. Consider a data-folder setting (planned with configurable game directories in Stage 4). |
| SQLite on network drives | WAL needs a local disk. The data folder is local by default. Revisit when the data folder becomes configurable. |
| Snap layouts on custom maximize button | Still not supported (Stage 6). |

## Next proposed stage: Stage 2, Minecraft installation engine hardening

1. Version coverage: legacy virtual assets (pre-1.7) and older natives, tested against a matrix of 1.8.9, 1.12.2, 1.16.5, 1.20.1, 1.21.x and 26.x.
2. A Downloads screen backed by a download queue and history (SQLite migration v2).
3. Install recovery: an atomic version directory swap, plus repair that reports exactly what was fixed.
4. Java: detection of installed runtimes, the compatibility matrix, and per-instance overrides with validation.
5. Push to GitHub and get CI green (needs the repository name).
