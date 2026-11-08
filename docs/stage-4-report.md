# Stage 4 report: mod loaders and instance folders

Branch: `feat/stage-4-loaders` (local, branched from `feat/stage-3-auth-java`, not pushed) · Date: 2026-11-08

## Summary

- **Fabric:**
  - The profile comes from Fabric Meta and is merged onto vanilla. Fabric Meta publishes no hash for
    `fabric-loader` and `intermediary`, so their SHA-1 comes from the Maven `.sha1` sidecar (same HTTPS
    origin) and is verified before use.
  - Version lists treat any release without a SemVer pre-release suffix as stable. Fabric Meta flags
    only the newest loader as "stable" (1 of 255), which would otherwise mislabel every older release.
- **NeoForge (native processors, as recommended in the Stage 0 spike):**
  - The official installer jar is downloaded and verified against its Maven `.sha1`. Its
    `install_profile.json` (spec 1 only) and `version.json` are read.
  - Every installer and runtime library goes through the verified download engine.
  - `/data/...` entries are extracted with path validation.
  - Mojang mappings come from the vanilla JSON (verified), and the processor that would download them
    itself is skipped.
  - Each client processor runs as `java -cp … Main args` with the managed runtime: no shell, no console,
    cancellable, 10-minute timeout. Declared output hashes are checked, and the `PATCHED` output must exist.
  - The engine is generic. NeoForge 21.1 (six processors: mappings, split, rename, patch) and NeoForge
    26.x (one `PROCESS_MINECRAFT_JAR` processor) both work without special cases.
- **Version layout:** each loader version gets `versions/<id>/<id>.jar` (a hard link to the vanilla jar),
  matching the official launcher. NeoForge's JVM arguments tell FML to ignore the jar named
  `${version_name}.jar`; with only the vanilla `1.21.1.jar` on the classpath, vanilla and patched classes
  would both have been loaded. Launch now resolves `inheritsFrom` chains from local files.
- **Instances:**
  - New instance has a loader choice (Vanilla / Fabric / NeoForge) and a loader-version picker. The
    newest stable version is preselected, betas are opt-in, and unsupported combinations are explained
    with creation disabled.
  - Instance details show the loader with a "Change version" dialog.
  - Install, verify and play are keyed by `vanilla:<mc>`, `fabric:<mc>:<v>` or `neoforge:<mc>:<v>`, and
    Downloads labels jobs accordingly ("NeoForge 26.3.0.101 · 26.3").
- **Custom game folder per instance** (migration v4):
  - Chosen with a native folder picker that runs in Rust, so the WebView gets no file-system permission,
    or typed in.
  - Validated: absolute, not a drive root, no `..` or alternate data streams, not inside Windows,
    Program Files or ProgramData, and not inside Night Client's managed folders or the vault.
  - Existing files are not moved. Deleting the instance never touches a custom folder.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **131 passed**, 0 failed, 2 ignored (network suites below) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `pnpm typecheck` / `pnpm test` | clean / **24 passed** |
| **Loader matrix** (`tests/loaders.rs`, real servers) | **4/4 pass** (table below) |
| Vanilla version matrix (Stage 2), re-run because launch preparation changed | **pass** (8 versions) |
| Desktop smoke test (debug build, 19 checks) | **19/19 pass** |
| Startup benchmark (release, two 5-run batches) | UI ready 290–317 ms; private memory 174–216 MB, then 175–184 MB; idle CPU ≤ 0.29 % |

| Loader | Install time | Libraries | Main class found in | Patched client |
|---|---|---|---|---|
| Fabric 0.20.1 · 1.21.1 | 0.6 s | 78 | classpath (`KnotClient`) | — |
| Fabric 0.20.1 · 26.3 | 0.5 s | 81 | classpath | — |
| NeoForge 21.1.265 · 1.21.1 | 10.7 s (6 processors) | 106 | module path (`BootstrapLauncher`) | 5.4 MB |
| NeoForge 26.3.0.101 · 26.3 | 2.6 s (1 processor) | 100 | module path (`fml.startup.Client`) | 37.6 MB |

Times are on the reference PC with vanilla files already cached.

## Bugs found and fixed during this stage

1. **NeoForge processor failed with `NoClassDefFoundError: GsonBuilder`.** The installer's library list
   deliberately contains several versions of the same artifact (Gson 2.8.9 and 2.10.1, several ASM
   versions). The resolver de-duplicated them as it does for a game classpath. Fixed with a
   non-deduplicating `resolve_all` for installer lists, plus a regression test.
2. **Every Fabric version except the newest showed as "beta".** Fixed as described above.
3. **Log redaction over-matched `code="auth_not_configured"`**, hiding error codes from diagnostics.
   OAuth `code=` values are now redacted only when they look like real codes: not part of another name,
   6+ characters, and not a lowercase identifier. Positive and negative tests were added. Log fields were
   renamed to `error_code`.
4. **Several smoke-test failures were test bugs**, not product bugs: regex escaping, and an assumption
   that installs would run when the loader matrix had already cached those versions.

## Not done or not verified

- **No modded (or vanilla) game has actually been started.** Fabric and NeoForge installs are verified up
  to a complete launch command whose main class and every classpath/module-path jar exist. Booting the
  game needs sign-in (R1). NeoForge in particular has runtime requirements (module layer, FML discovery)
  that only a real launch can confirm.
- **The NeoForge CI cross-check** against the official installer, recommended in the spike, is not
  implemented. It needs CI, which needs a GitHub remote.
- **Older NeoForge lines** (20.2–20.6 for 1.20.x) are mapped by the version rule but weren't installed
  in the matrix.
- **Data-folder relocation is not implemented** (deferred). Moving gigabytes safely needs copy, verify,
  switch and rollback, which belongs with Stage 7 hardening. Per-instance custom game folders cover the
  "worlds on another drive" case. The roaming-profile risk remains for libraries and assets.
- **Forge and Quilt** are not implemented. The install-profile model and loader dispatch are written to
  accept them later.
- **Memory** is still above the 150 MB target.
- **Test artifacts:** the loader tests cached Fabric 0.20.0/0.20.1 and NeoForge 21.1.265/26.3.0.101 files
  in your data folder (valid files, reused later). All smoke-test instances and trash folders were removed.

## Next proposed stage: Stage 5, Modrinth integration

Search and discovery with filters and pagination, compatibility (game version and loader), install and
remove, required-dependency resolution, update detection, installed-mod metadata (migration v5),
local JAR import with warnings, API caching and rate-limit handling with a proper `User-Agent`, and
SHA-512 verification of downloaded mods.
