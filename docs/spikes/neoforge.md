# Spike: NeoForge installation (Stage 0)

> **Outcome (Stage 4):** option B is implemented in `crates/nc-launcher/src/loaders/neoforge.rs` and
> verified against real servers. NeoForge 21.1.265 (six client processors) installs in 10.7 s, and
> 26.3.0.101 (a single `PROCESS_MINECRAFT_JAR` processor, a format that appeared after this spike)
> installs in 2.6 s.
>
> One finding the static analysis missed: the installer library list contains **several versions of
> the same artifact**, because different processors use different Gson and ASM versions, so it must not
> be de-duplicated. See `docs/stage-4-report.md`. Booting the game is still unverified (needs sign-in).

**Question:** run the official installer headless, or execute its processors natively?

## What was examined

The official `neoforge-21.1.265-installer.jar` (Minecraft 1.21.1) from `maven.neoforged.net`.
`install_profile.json` and `version.json` were extracted and read. **The installer was not executed**
in Stage 0, so the runtime behaviour below has not been tested yet.

## Findings

| Item | Value |
|---|---|
| `install_profile.json` spec | 1 |
| Processors total / client-side | 10 / 6 |
| Client processors | `MCP_DATA` (extract mappings from NeoForm), `DOWNLOAD_MOJMAPS`, `MERGE_MAPPING`, `jarsplitter`, `AutoRenamingTool`, `binarypatcher` |
| Installer libraries | 70, **all with SHA-1** |
| `version.json` | `inheritsFrom: 1.21.1`, main class `cpw.mods.bootstraplauncher.BootstrapLauncher`, 47 libraries, all with URLs |
| Patched client output | `[net.neoforged:neoforge:21.1.265:client]` (`data.PATCHED`) |
| Declared processor `outputs` hashes | **none**, so the patched jar cannot be verified by hash |
| JVM args | module path built from `${library_directory}` and `${classpath_separator}`. `nc-core` already substitutes both. |

## Options

**A. Run the official installer headless** (`java -jar installer.jar --install-client <dir>`)
- + Least code. NeoForge maintains the logic.
- − It writes its own `launcher_profiles.json` expectations and its own directory layout.
  We get no per-step progress, no cancellation, and no reuse of our verified download cache.
  It downloads libraries itself, outside our integrity checks and retry policy.

**B. Execute processors natively** (the approach Prism Launcher and Modrinth App use)
1. Download installer libraries and version libraries through our engine (SHA-1 verified).
2. Extract `/data/client.lzma` and other `data` entries from the installer jar (zip-slip-safe extraction).
3. Resolve `{TOKENS}` and `[maven:coords]` in each processor's args.
4. Provide `{MOJMAPS}` ourselves from `downloads.client_mappings` in the vanilla JSON (verified), and
   skip `DOWNLOAD_MOJMAPS`, so processors make no unverified network access.
5. Run each processor as `java -cp <jar;its classpath> <Main-Class> args…`. No shell, managed runtime,
   cancellable, with per-step progress.
6. Write the install marker only after every processor exits 0 and the `PATCHED` output exists.
- + Consistent layout, verified downloads, progress and cancellation.
- − More code (~400 lines). The processor protocol can change (tracked by the spec version).

## Recommendation

**Option B**, gated on `spec == 1`. An unknown spec fails with a clear "unsupported NeoForge
installer format" error instead of guessing. Run option A once in CI as a cross-check that both produce
identical `neoforge-*-client.jar` hashes for one pinned version.

## Security notes

Processors are NeoForge's own Java tools, run with our managed Java runtime inside the data directory.
This is inherent to supporting NeoForge. Mitigations: no shell, an argument vector built from validated
tokens, `JAVA_TOOL_OPTIONS` stripped, and outputs confined to the data directory via `safe_join`.

## Remaining unknowns (Stage 4)

- Wall-clock time and peak memory of the processors on the reference PC.
- Whether older NeoForge versions (20.x for 1.20.x) use the same processor set.
