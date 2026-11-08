# Architecture

```
┌──────────────────────── apps/desktop (Tauri 2) ────────────────────────┐
│  React UI (src/)            IPC (invoke / events)       src-tauri/     │
│  tokens · components ──────────────────────────────▶  commands.rs     │
│  zustand stores      ◀── install://progress (≤10 Hz)  state.rs        │
│                      ◀── game://log (batched 100 ms)                   │
└──────────────────────────────────────────┬─────────────────────────────┘
                                           │ plain Rust calls
┌──────────────────── crates/nc-launcher (services, async) ──────────────┐
│ download   meta   install   java   auth   process   fsutil   layout    │
└──────────────────────────────────────────┬─────────────────────────────┘
                                           │
┌──────────────────── crates/nc-core (pure, no I/O) ─────────────────────┐
│ manifest   rules   maven   libraries   profile   launch   java   paths │
│ secret                                                                 │
└────────────────────────────────────────────────────────────────────────┘
```

## Principles

1. **Pure core.** `nc-core` has no Tokio, no network and no file system. It is fast to test and reusable.
2. **Thin shell.** `src-tauri` validates input, calls a service and maps errors to `{ code, message }`.
   No business logic lives there.
3. **Untrusted metadata.** Version JSON, loader profiles, runtime manifests and (later) mod metadata
   are treated as hostile: every path is validated, Maven coordinates are checked segment by segment,
   and download URLs must be HTTPS.
4. **Secrets are typed.** Tokens live in `nc_core::Secret`, whose `Debug`/`Display` are redacted
   and whose buffer is zeroed on drop. `LaunchCommand` keeps its secrets separately so it can always
   produce redacted arguments, and zeroes its arguments (which carry the access token) on drop.
5. **Throttle IPC.** Progress is sampled at 10 Hz and logs are batched every 100 ms, so heavy work never
   floods the WebView.

## Data directory

`%APPDATA%\NightClient` always holds the database (`night-client.db`) and, when the user moved the game
data, `data-location.json` pointing at it. Logs and the token vault are machine-local in
`%LOCALAPPDATA%\NightClient`. The game data below lives in `%APPDATA%\NightClient` by default, or
wherever the user moved it (Settings → Storage).

| Path (under the game-data root) | Contents | Shared? |
|---|---|---|
| `cache/` | version manifest (offline fallback) | — |
| `versions/<id>/` | version JSON, client jar, `.installed` marker | across instances |
| `libraries/` | Maven-layout libraries | across instances (acts as the cache) |
| `assets/` | `indexes/`, `objects/`, `log_configs/` | across instances |
| `runtimes/<component>/` | official Mojang Java runtimes | across instances |
| `instances/<name>/` | game directory, `.natives/<version>/` | per instance |

A version counts as installed only when its `.installed` marker exists. The marker is written atomically
after every file has verified.

**Moving the game data** (`nc_launcher::relocate`, `commands::data::relocate_data`):
- Refused while a game runs, downloads are queued, or a custom game folder lies inside a managed folder.
  While a move runs, installs, launches and instance or mod changes are refused.
- Each managed folder is renamed (same drive) or copied with size checks (another drive; links are
  refused). Any failure renames back and deletes copies.
- `data-location.json` is written before the old copies are deleted, then the app restarts. A crash at
  any point leaves one complete copy that the saved location points to.
- Targets must be local (Java runtimes are executed from here), empty or new, outside Windows folders
  and not nested with the current folder. The default folder is always accepted.

## Download engine invariants

- A destination file is either absent or complete and verified (`.part` → verify → `rename`).
- `.part` files are resumed with `Range`. A complete `.part` (interrupted just before the rename) is
  verified without a network request.
- Retryable failures: network errors, timeouts, 408/429/5xx, size or hash mismatch. Not retryable:
  other 4xx, I/O errors, insecure URLs, insufficient disk space.
- The first non-retryable failure cancels the whole batch. The root cause is reported, not the
  cancellations it triggered.

## Installation, repair and launch pre-flight

- **Install marker semantics:** `versions/<id>/.installed` is deleted when an install or repair
  starts touching files and written atomically only after every file verifies. A version without the
  marker is treated as not installed, and Play re-installs it.
- **Repair report:** the download engine returns the destinations it actually fetched. During a repair
  these are the missing or damaged files, stored (up to 200) in `download_history.repaired`.
- **Legacy assets:** `virtual` indexes are laid out in `assets/virtual/<index>` at install time.
  `map_to_resources` indexes are copied into `<instance>/resources` at launch, hard-linking when
  possible. Names come from the hash-verified index and are still validated with `safe_join`.
- **Pre-flight:** `prepare_launch` checks the client jar, every library and the Java runtime exist
  before building the command.
- **Queue:** `nc_launcher::queue::Queue` holds pure state: one active job, FIFO pending,
  de-duplicated. The app's worker (`commands/downloads.rs`) waits on a `Notify`, runs one job at a
  time, records history and emits `downloads://queue`, `install://progress` and `install://finished`.

## Java

- `nc_core::java`: requirement from `javaVersion` (Java 8 if absent), the compatibility matrix,
  `release`-file and `-version` parsing, and PE-header architecture detection.
- `nc_launcher::javadetect`: scans known locations and inspects without executing when a `release`
  file exists.
- A per-instance override is stored in `instances.java_path` and validated on save and at launch.
  An incompatible runtime is refused (`java_incompatible`); a risky one is allowed with a warning in the log.

## Accounts and tokens

| What | Where | Lifetime |
|---|---|---|
| Microsoft refresh token | DPAPI vault `%LOCALAPPDATA%\NightClient\vault\<uuid>.ncv` | until revoked or the account is removed |
| Minecraft access token | memory (`AppState.sessions`) | about 24 h, renewed 5 min before expiry |
| Name, UUID, last used, active | SQLite `accounts` | until the account is removed |

`commands::account::active_session` is the only way to get a token for launching. It returns the
in-memory session or renews it through `nc_launcher::accounts::refresh` under a lock. Nothing
secret is ever returned over IPC.

## JVM arguments

`nc_core::jvmargs::parse` splits user text without a shell and validates it against an allowlist.
The stored vector is validated again in `prepare_launch`. Memory is always set by the launcher.

## Mod loaders

**Install key** (`nc_core::loader::InstallKey`): `vanilla:<mc>`, `fabric:<mc>:<v>`,
`quilt:<mc>:<v>`, `neoforge:<mc>:<v>` or `forge:<mc>:<v>` (the Forge version without its `<mc>-`
prefix). It is the download-queue target and maps to a **launch id**: `<mc>`,
`fabric-loader-<v>-<mc>`, `quilt-loader-<v>-<mc>`, `neoforge-<v>` or `<mc>-forge-<v>`.

`nc_launcher::loaders::install` dispatches on the key. Each loader installs vanilla first, then writes
`versions/<launch id>/` with the loader JSON (which `inheritsFrom` vanilla), a hard link of the vanilla
jar named `<launch id>.jar`, and finally `.installed`. At launch, `loaders::resolve_version` merges the
`inheritsFrom` chain from local files with `nc_core::profile::merge`: child scalars win, arguments are
appended, and child libraries override parent libraries with the same `group:artifact[:classifier]`.

- **Fabric and Quilt** (`loaders/fabric.rs`, `Flavor`): profile from Fabric Meta v2 or Quilt Meta
  v3, which share a format. Jars without a hash use the Maven `.sha1` sidecar. Quilt instances also
  accept Fabric mods (`nc_core::loader::mod_loaders`).
- **NeoForge and Forge** (`loaders/neoforge.rs`, `Dist`): runs the installer's processors natively,
  using `nc_core::installprofile` for the model and argument resolution. Installer libraries are resolved
  with `libraries::resolve_all`, without de-duplication, because processors need different versions of
  the same artifact. Forge 1.17+ installers use the same
  `install_profile` spec 1. Forge 1.12.2 (build 14.23.5.2851+) to 1.16.5 use spec 0, which has the
  same shape, but lists the Forge jars without a URL and ships them under `maven/` in the installer;
  those are extracted and checked against the SHA-1 the profile declares. After the processors run,
  every runtime library must exist. Older 1.12.2 builds and earlier versions use the legacy installer
  format and are not offered. See
  [spikes/neoforge.md](spikes/neoforge.md).

## Modrinth

`nc_launcher::modrinth`:
- `api`: the client. It sends the `User-Agent`, honours rate-limit headers (pauses, one 429 retry),
  caches responses (TTL, at most 256 entries), and validates ids, CDN-only file URLs and file names.
- `plan`: resolves required dependencies through the `VersionSource` trait, so tests use a fake catalogue.
- `mods`: the content folders. It scans with SHA-1/SHA-512, installs with both hashes verified, toggles
  `.disabled`, removes files, and inspects and imports local jars (mods only).

Content kinds (`nc_core::content::ContentKind`, Stage 12) are mods, resource packs and shader packs.
Each has a folder (`mods`, `resourcepacks`, `shaderpacks`), a file extension (`.jar`, `.zip`) and the
Modrinth loader tags it accepts on an instance (`modrinth_loaders`): the instance's mod loaders for mods,
`minecraft` for resource packs, and `iris`/`optifine`/`vanilla` for shaders (only `vanilla`, meaning
core-shader packs, on a vanilla instance). Search, version lookup, update checks and `pick_best` take that
list of tags. Packs are single files, so they skip dependency resolution.

The folder is the source of truth. `instance_mods` (migration v5; `kind` column since v7) caches metadata
and is re-synced on every listing. The app layer (`commands/mods.rs`) re-validates compatibility before every install and
blocks changes while the game runs.

### Turning packs on in the game (Stage 15)

`nc_core::gameopts` edits the game's own settings as text and keeps every other line byte for byte (and
the file's line endings):
- **Resource packs:** the `resourcePacks:[...]` JSON line of `options.txt`. Ids are `file/<name>` from
  Minecraft 1.13 on (`uses_file_prefix`), the bare name before. Turning a pack on puts it last, which is
  the top priority.
- **Iris shader packs:** `shaderPack` and `enableShaders` in `config/iris.properties` (Java properties;
  non-ASCII written as `\uXXXX`, a Latin-1 file is read as Latin-1).

`nc_launcher::gameopts` reads (1 MiB cap) and writes (temp file, then rename) those files. A new
`options.txt` starts with `version:<data version>` from the client jar's `version.json` when it's
installed, so the game doesn't data-fix it as an old file. When a pack is updated or removed, its entry
follows the new file name or is dropped (`follow_pack`). Changes are refused while the game runs, because
the game rewrites `options.txt` on exit. Shader packs can only be turned on with Iris installed.

On a vanilla instance, "shader packs" from Modrinth are core-shader packs, which the game loads as
resource packs. Since Stage 15 they install into `resourcepacks/` (`stored_kind`).

### Modpack updates (Stage 14)

An import records the pack's origin and every file it wrote (downloaded files and overrides, an override
replacing a file at the same path) with its SHA-1. A pack imported from a file is looked up on Modrinth by
SHA-512, so it can be updated too when Modrinth lists it.

`nc_launcher::modrinth::packupdate` compares, per path (case-insensitive), the recorded hash, the hash on
disk now (or of `<path>.disabled`) and the new pack's:

| On disk | Recorded | In the new pack | Action |
|---|---|---|---|
| missing | — | yes | add |
| missing | yes | yes | keep (the user deleted it) |
| = new | any | yes | unchanged |
| = recorded | yes | yes | replace (disabled stays disabled) |
| other | any | yes | keep (the user's) |
| = recorded | yes | no | remove |
| other | yes | no | keep |

Kept files stay tracked with their install-time hash, so later updates still see them as the user's.
The new overrides are extracted and the new files downloaded (size, SHA-1, SHA-512) into
`<game>/.nc-update` first. Files are only moved into place once everything is verified, so a failed or
cancelled update changes nothing. The update may change the Minecraft and loader versions, never the
loader. The app then records the new version and files, and patches the instance.

### Modpacks (Stage 13)

- `nc_core::mrpack` parses and validates `modrinth.index.json` before anything is written: format 1,
  game `minecraft`, exactly one supported loader (Fabric, Quilt, NeoForge, Forge) or none, at most 4000
  files and 8 GiB, unique relative paths (case-insensitive) outside `.natives`, hex SHA-1/SHA-512, and
  HTTPS download URLs on `cdn.modrinth.com`, `github.com`, `raw.githubusercontent.com` or `gitlab.com`
  (no credentials, no port). Files marked `unsupported` on the client are left out.
- `nc_launcher::modrinth::modpack`: `read_pack` (index capped at 16 MiB), `fetch_pack` (a Modrinth
  `.mrpack`, verified, cached as `cache/modpacks/<sha1>.mrpack`), `download_files` (download engine, then
  SHA-512) and `extract_overrides` (`overrides/`, then `client-overrides/` winning per path; same path
  rules, symlinks refused, 2 GiB total cap, temp file then rename).
- `commands/modpacks.rs`: a preview (Modrinth project or native file picker) keeps the validated pack
  behind a one-time token, so the WebView never supplies a path. Import creates the instance, emits
  `modpack://progress`, and discards the instance (row, then folder to trash) if anything fails or the
  import is cancelled. The game and loader are queued for download as for a new instance.

## Instance game folders

By default an instance's game folder is `<data>/instances/<dir_name>`. A custom folder
(`instances.game_dir`, migration v4) is validated by `instance::validate_custom_game_dir`. Extracted
natives always stay under `<data>/instances/<dir_name>/.natives`. Deleting an instance moves only the
default folder to trash; a custom folder is never touched.

## Local database (`nc-store`)

- `<data>\night-client.db`, SQLite (bundled), WAL, `foreign_keys=ON`, 5 s busy timeout.
- Migrations live in `crates/nc-store/migrations/NNNN_name.sql` and are registered in `migrations.rs`.
  They are forward-only, run one transaction per migration together with the `user_version` bump, and
  versions must be contiguous.
- Before an existing database is migrated it is copied with `VACUUM INTO` to `night-client.db.v<N>.bak`.
- A schema newer than the build is refused (`NewerSchema`) and the file is left untouched. The app still
  starts, shows a banner, and keeps Diagnostics available.
- `STRICT` tables with `CHECK` constraints mirror the Rust validation.
- v8 (Stage 14): `instance_modpack` (the pack's Modrinth project, version, name, icon) and
  `instance_modpack_files` (each file the pack put in the game folder, with its SHA-1 then). Both
  cascade with the instance.
- Settings are one JSON document per key, using `#[serde(default)]`, so new settings need no migration.
  A corrupt value falls back to defaults.
- **Never** stores tokens. Account metadata arrives in Stage 3. The refresh token goes to DPAPI, not SQLite.

## Logging

`nc_launcher::logging::init` sets up a daily rotating file in `%LOCALAPPDATA%\NightClient\logs`
(7 files), a non-blocking writer, and a regex redaction pass on every line. Debug builds also log to
stderr. `Secret` remains the primary defence. The regexes are a safety net and have tests for both
leaks and false positives.

## IPC permissions

`apps/desktop/src-tauri/build.rs` declares every command in an `AppManifest`. Tauri then requires an
`allow-<command>` permission per command, and `capabilities/main.json` grants them to the `main` window
only. **Adding a command means adding it to `build.rs`, the capability, `lib.rs` and `src/lib/ipc.ts`.**

## Frontend

```
src/
  App.tsx            shell: title bar, sidebar, animated route outlet, global event subscriptions
  stores/app.ts      Zustand: app info, settings, account, instances, versions, install, running games, logs
  stores/router.ts   in-memory router with history (Alt+← and mouse back button)
  stores/toast.ts    toast queue (at most 4)
  lib/ipc.ts         the only file that names commands/events
  lib/actions.ts     cross-screen flows (install-if-needed → launch)
  components/        Button, ui primitives (Card, Dialog, Toggle, MemorySlider, states…), Sidebar, PlayButton,
                     LogView (virtualized), CommandPalette, Toaster, InstanceArt, TitleBar
  screens/           Home, Instances, NewInstance, InstanceDetail, Discover, Downloads, Accounts, Settings, Diagnostics
e2e/                 DevTools-protocol driver, UI smoke test, axe-core accessibility audit (debug builds only)
```

Rules:
- **Zustand selectors must return stable values.** Select the stored object and derive arrays in
  `useMemo`. A selector that builds a new `[]` or `{}` re-renders forever under Zustand v5
  (React #185).
- **Colour tokens live on `:root` in `styles/app.css`**, with measured contrast ratios in comments.
  `fg-faint` is the dimmest text colour allowed, at 4.8:1 or better on every surface.
- **Motion** is 100–220 ms. It is off when the OS asks for reduced motion or the "Reduce motion"
  setting is on (`MotionConfig` plus a `.reduce-motion` class).
- **Global shortcuts** are handled in `App.tsx` and documented in Settings → Keyboard shortcuts.
- **Results that finish off-screen** (installs, game crashes) are reported as toasts. Errors use
  `role=alert`.
- **Snap layouts:** the title bar's maximize button calls `show_snap_layouts` after a 500 ms hover. That
  command sends Win+Z only when the window is focused and not maximized.

Screens for features that don't exist yet are not shown, so there are no placeholder controls.

## Memory

- The Rust process idles at about 11 MB; the rest is WebView2.
- WebView2 renders in software unless Settings → Hardware acceleration is on (read once at startup,
  `--disable-gpu`). This saves the GPU process's ~70 MB, and frame times for this UI were unchanged
  in measurement (docs/benchmarks).
- `memory.rs` sets WebView2's low memory-usage target while the window is minimized or hidden for a
  game, and normal again when it is shown.

## Updates

- `commands/update.rs` wraps `tauri-plugin-updater`. The public key and feed URL are compiled in
  (`NIGHT_CLIENT_UPDATE_PUBKEY`, `NIGHT_CLIENT_UPDATE_ENDPOINT`) by the release workflow. Without
  them the plugin is not registered and Settings explains that the build has no update channel.
- `check_for_update` stores the found update in managed state. `install_update` downloads it,
  verifies the minisign signature (`requireSignedVersion` also binds it to the announced version)
  and runs the NSIS installer in passive mode. The installer closes the app and starts the new
  version.
- Installs are refused while games run, downloads are queued or data is being moved.
- The WebView has no updater permission; only these two app commands exist.
- The feed is `latest.json` on a fixed `update-feed` GitHub release, updated by a manual workflow
  after a release is published (docs/releasing.md).

## Crash reports and diagnostics

- A panic hook (`install_crash_reporter` in `lib.rs`) logs the panic and writes
  `logs/crash-<time>-<pid>.txt` through `nc_launcher::report` (redacted, newest 10 kept).
- `take_crash_notice` reports new crash files once, and the UI shows a toast.
- Diagnostics → Export… saves a zip (native save dialog, Rust side) with `diagnostics.json`, the
  newest 5 logs and the crash reports. Every line is redacted again, and large files are cut to
  their last 4 MiB. The database, the vault and game files are never included.

## First run

`components/Welcome.tsx` is a modal shown while `settings.onboarded` is false. Finishing, skipping
or pressing Esc sets it, and Settings → About can show it again. It also holds the
automatic-update-check choice.

## Security review

See [security-review.md](security-review.md) for each requirement in the brief, how it is met, and
where it is tested.

## Planned additions

- Authenticode signing of the installer (needs a code-signing certificate).
