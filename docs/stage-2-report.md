# Stage 2 report: installation engine hardening

Branch: `feat/stage-2-install-engine` (local, branched from `feat/stage-1-foundation`, not pushed) · Date: 2026-11-08

## Summary

- **Version coverage:** installing and preparing a launch now works from 1.5.2 to 26.3, including both
  legacy asset layouts. Pre-1.6 `map_to_resources` assets are copied into `<instance>\resources` at
  launch, and 1.6–1.7 `virtual` indexes are laid out in `assets\virtual\<index>`. `${game_assets}` is
  now computed per version instead of hard-coded.
- **Repair you can trust:** the install marker is removed before files are touched and rewritten only
  after every file verifies, so an interrupted repair can't leave a version that looks complete. Repairs
  report exactly which files were missing or damaged (up to 200 paths, plus a total).
- **Launch pre-flight:** before Java starts, the client jar, every library and the runtime are checked
  for existence. A damaged install fails with `files_missing` ("use Verify files") instead of a
  `ClassNotFoundException` inside the game.
- **Download queue:** installs and verifies are queued (one active job, FIFO, de-duplicated by
  version). Pending jobs can be removed and the active one cancelled. A background worker drains the queue.
- **Download history:** SQLite migration v2 adds `download_history` (last 200 jobs), shown on the new
  Downloads screen with status, duration and the repaired-file list.
- **Java management:**
  - **Detection:** Night Client's own runtimes, `JAVA_HOME`, `PATH`, common vendor folders under
    Program Files, and the official Minecraft Launcher's runtimes.
  - **Inspection:** version from the JDK `release` file, architecture from the `java.exe` PE header. No
    binary is executed unless it lacks a `release` file; then `-version` runs with a 5 s timeout and no
    console window.
  - **Compatibility levels:** recommended / compatible / may not work / incompatible. 32-bit and too-old
    runtimes are refused.
  - **Per-instance Java:** an instance can use a specific runtime, validated when saved and again at launch.
- **Housekeeping:** at startup, `*.part` and `*.tmp` leftovers older than 24 h are removed. Symlinks
  and junctions are never followed.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **103 passed**, 0 failed, 1 ignored (the network matrix below) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `pnpm typecheck` / `pnpm test` | clean / **22 passed** |
| **Version matrix** (`--ignored`, real Mojang servers) | **8/8 pass**: 1.5.2, 1.6.4, 1.8.9, 1.12.2, 1.16.5, 1.20.1, 1.21.1, 26.3 (see table) |
| Desktop smoke test (debug build, 19 checks) | **19/19 pass** (after fixing the two issues below) |
| Startup benchmark (release, 5 runs) | UI ready **275–283 ms**, private memory **175–177 MB**, idle CPU **0 %** |

### Version matrix

The matrix installs each version, prepares the launch command (no process started, so nothing is
launched without authentication), then checks that the main class is on the classpath, legacy natives
were extracted, and legacy assets are laid out.

| Version | Java | Libraries | Native DLLs extracted | Asset layout | Main class |
|---|---|---|---|---|---|
| 1.5.2 | 8 | 10 | 9 | map_to_resources | `net.minecraft.launchwrapper.Launch` |
| 1.6.4 | 8 | 18 | 9 | virtual | `net.minecraft.client.main.Main` |
| 1.8.9 | 8 | 35 | 14 | hashed | ″ |
| 1.12.2 | 8 | 35 | 11 | hashed | ″ |
| 1.16.5 | 8 | 41 | 16 | hashed | ″ |
| 1.20.1 | 17 | 64 | 0 (LWJGL 3 extracts itself) | hashed | ″ |
| 1.21.1 | 21 | 70 | 0 | hashed | ″ |
| 26.3 | 25 | 74 | 0 | hashed | ″ |

### Smoke test highlights

- The 1.12.2 client jar was corrupted (one flipped byte) and a library deleted. **Verify files**
  repaired exactly those 2 files, and the Downloads history lists both paths. A second verify reports
  every file matching.
- Queue: two jobs queued, a duplicate request returns the same job id, the pending job was removed
  from the UI and the active job cancelled from the UI. History records "Cancelled", and the cancelled
  version is **not** marked installed.
- Java: `notepad.exe` is rejected (`java_invalid`), Java 21 is rejected for 26.3, which needs 25
  (`java_incompatible`), relative paths are rejected, and Java 8 is accepted for 1.12.2 as
  "recommended". The picker disables incompatible runtimes.

## Issues found and fixed during this stage

1. **The Java picker hid Night Client-managed runtimes.** On a PC with no other Java installed you
   couldn't choose, say, the Java 21 runtime for a 1.20.1 instance. All detected runtimes are now listed
   and rated.
2. **The first queue smoke test raced the network.** A 1.7.10 install finished before the test could
   cancel it. The test now lowers parallel downloads to 1 and uses a larger uninstalled version.
   This was a test problem, not a product bug.

## Not done or not verified

- **Sign-in and real launch are still blocked** (Mojang app-ID approval, R1). The Java override, the
  `files_missing` pre-flight and pre-1.6 resource copying are exercised by the matrix and unit tests up
  to the launch command, not by starting the game.
- **CI still not run** (no GitHub remote).
- **Windows registry Java entries** are not read. Detection covers the standard install folders, which
  is where the registry points for the common vendors. A registry scan can be added if users report
  missed runtimes.
- **JVM argument editing** (validated) is not built yet. It's planned with the rest of Java management
  in Stage 3.
- **Memory** is still ≈176 MB against the < 150 MB target. One of 8 benchmark runs this stage spiked to
  218 MB (cold WebView2 GPU process), as also seen once in Stage 1.
- **Test leftovers:** the smoke and matrix runs left 1.5.2–1.19.2 game files in your data folder's
  shared cache, about 1.2 GB. They're valid game files and reused by future installs. Partial `.part`
  files from the cancelled 1.19.2 job are removed automatically after 24 h.

## Next proposed stage: Stage 3, authentication and Java management

1. DPAPI token vault, persisted sign-in, token refresh, multiple accounts and switching (migration v3 for
   non-sensitive account metadata).
2. End-to-end Microsoft sign-in once Mojang approves the app ID, then the first real game launch,
   checking hide-while-playing and play-time recording.
3. JVM argument editing with validation (block `-cp`, `-jar`, agents and other dangerous flags).
4. Push to GitHub and get CI green (needs the repository name).
