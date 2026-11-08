# Stage 5 report: Modrinth integration

Branch: `feat/stage-5-modrinth` (local, branched from `feat/stage-4-loaders`, not pushed) · Date: 2026-11-08

## Summary

- **Modrinth client** (`nc_launcher::modrinth::api`):
  - **Requests:** an identifying `User-Agent` on every request, 10 s connect and 30 s total timeouts.
  - **Rate limits:** the `X-Ratelimit-Remaining`/`-Reset` headers are honoured. When the budget is
    spent, the next request waits for the window to reset. A `429` waits and retries once, then reports
    `rate_limited`.
  - **Cache:** a bounded, time-limited in-memory response cache (search 5 min, versions 10 min,
    projects 30 min, version 1 h, categories 1 day; at most 256 entries).
  - **Untrusted responses:** ids and slugs are validated before they go into a URL path. File URLs must
    be HTTPS on `cdn.modrinth.com`. File names must be safe `.jar` names. Project bodies (Markdown/HTML)
    are never rendered; only the plain description is shown.
- **Discover screen:**
  - Search is scoped to an instance's loader and game version. Server-only mods are filtered out.
  - Category chips, five sort orders, debounced search, "Load more" pagination, Modrinth icons (lazy,
    no referrer, CDN-only), an "Installed" marker, and an "open on Modrinth" button that can only open
    `modrinth.com/mod/<validated slug>`.
- **Install plan** (`modrinth::plan`):
  - Before anything is written, the user sees the chosen version (releases preferred over betas), every
    required dependency (transitively, cycle-safe, capped at 64), dependencies already installed, required
    projects with no compatible version, and declared incompatibilities.
  - A risky plan needs an explicit "I understand" checkbox.
  - The dialog says that only declared conflicts can be detected.
- **Installing:** each version is re-fetched and re-checked for loader and game-version compatibility
  on the Rust side. Each file is downloaded through the verified engine (SHA-1 and size), then SHA-512 is
  checked. A mismatch deletes the file.
- **Mods tab** (Fabric/NeoForge instances):
  - Installed mods with icon, version, size and badges ("Dependency", "Unverified file", "Update: x").
  - Enable/disable renames the file to and from `.jar.disabled`.
  - Remove warns which mod required this one.
  - Check for updates and Update all use Modrinth's `/version_files/update`. The new file is verified
    before the old one is deleted, and disabled mods stay disabled.
  - Every change is blocked while the game is running.
- **Folder sync:** listing re-syncs the database with the folder.
  - Rows for deleted files are dropped.
  - Unknown or replaced jars are identified on Modrinth by SHA-512, or recorded as "local" when offline
    or unknown.
  - Enabled state follows the disk, and non-jar files are ignored.
- **Local import:**
  - A native file picker runs in Rust, so the WebView never supplies a path.
  - Only `.jar` files of 256 MB or less that are valid zips are accepted. The jar's own metadata
    (`fabric.mod.json`, `neoforge.mods.toml`, `mods.toml`) is read without executing anything, and a mod
    declared for another loader is refused.
  - Files not found on Modrinth are labelled "unverified", with an explanation.
- **Storage:** migration v5 adds `instance_mods` (metadata only, cascade-deleted with the instance).

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **143 passed**, 0 failed, 3 ignored (network suites) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `pnpm typecheck` / `pnpm test` | clean / **24 passed** |
| Real Modrinth API test (`tests/modrinth.rs --ignored`) | **pass**: search → plan (Iris pulls in Sodium) → install both (SHA-512 verified) → identify by hash → update check |
| Desktop smoke test (debug build, real Modrinth, 21 checks) | **21/21 pass**, on three runs after the fixes below |
| Startup benchmark (release, two 5-run batches) | UI ready 292–326 ms; private memory 175–207 MB; idle CPU ≤ 0.1 % |

Smoke-test highlights:
- **Discover:** an empty state explains why there are no results for vanilla-only users. 14,725 results
  for Fabric 1.21.1, and pagination works.
- **Iris install:** the plan shows "Sodium · required by Iris Shaders", and both jars land on disk.
- **Mods tab:** disable and enable rename the files; the remove dialog warns that Iris requires Sodium.
- **Folder sync:** a jar deleted by hand disappears from the list, a jar copied by hand is identified
  on Modrinth by hash, and a `.txt` file is ignored.
- **Real update:** Sodium 0.8.12 was installed deliberately. The update to 0.8.13 was detected and
  applied, and the old file was removed.
- **Raw IPC refusals:** a `../` category, mods for a vanilla instance, `../../evil.jar`, a NeoForge build
  for a Fabric instance, and a non-slug "open page" request were all refused.

Mock-server tests cover the `User-Agent` and search facets, cache hits, a 429 wait-and-retry,
pausing for an exhausted budget, a persistent 429, not-found handling, ids rejected before any request,
and hash lookup. Unit tests cover dependency resolution (transitive, cycle, optional skipped, installed,
missing, conflicts), version preference, mods-folder operations and jar inspection.

## Bugs found and fixed during this stage

1. **The remove-dialog dependency warning looked up the relation backwards** and never showed. It's
   fixed, and the smoke test checks it.
2. **The search field collapsed to its icon**, because a shared `w-full` overrode width utilities on
   selects. The same bug was latent in Settings' "Parallel downloads". Both are fixed with sized wrappers.
3. **An external `<a target=_blank>` would not open the system browser in Tauri.** It's replaced by a
   Rust command restricted to `modrinth.com/mod/<slug>`.
4. One smoke failure was test timing (clicking a switch while it was disabled during a save), not a
   product bug.

## Not done or not verified

- **No game with mods has been launched** (sign-in blocked, R1). Whether the installed mods load is unverified.
- **Local import through the UI** wasn't automated, because the native file dialog can't be driven by
  the test. Its validation is covered by unit tests.
- **Updates don't re-resolve new dependencies** that a newer version may add. The next install plan or
  check would surface them. Planned for Stage 7.
- **Optional dependencies aren't offered.** Only required ones are installed.
- **Resource packs, shader packs, modpacks and CurseForge** are out of scope (as the brief requires for
  modpacks and CurseForge).
- **Memory** is still above target.
- **CI** has still not run.

## Next proposed stage: Stage 6, premium UI refinement

Visual and interaction polish across all screens:
- Instance cards and art, skeleton and transition consistency, keyboard shortcuts and a command palette.
- Windows 11 snap layouts on the custom maximize button.
- An accessibility audit (focus order, contrast, screen-reader labels) and toasts for background results.
- A large-list virtualisation pass for long mod lists and logs.
- Settings for reduced motion and density.
