# Stage 7 report: security, testing and optimization

Branch: `feat/stage-7-hardening` (local, branched from `feat/stage-6-ui`, not pushed) · Date: 2026-11-08

## Summary

- **Security review** of every requirement in the brief, written up in
  [security-review.md](security-review.md) with the tests that cover each one. Fixes it led to:
  - **Secrets are wiped from memory.** `Secret` and `LaunchCommand` (whose arguments carry the access
    token) zero their buffers when dropped (`zeroize`, already in the dependency tree). The vault wipes
    decrypted bytes on its error path.
  - **NeoForge installer extraction** fails with an error above a 512 MiB total instead of silently
    truncating a file.
  - **Mod ids read from jars** are kept only if they look like real ids. A 5,000-character or HTML id is
    dropped, and the loader is still detected.
  - **Custom Java paths** on network shares or device paths (`\\server\…`, `\\?\…`) are refused,
    because checking a runtime executes it.
  - **Event permission narrowed** from `core:event:default` to listen/unlisten. The WebView never emits.
- **Dependency audits:** `cargo deny` (advisories, licences, bans, sources) and `pnpm audit` are both
  clean. `pnpm audit` was added to CI.
- **Mod updates install new dependencies.**
  - If a newer version starts requiring another mod, it is resolved like an install plan and installed
    with it.
  - Missing dependencies are reported in the Mods tab.
  - Resolution happens before download, so a network failure leaves the old version in place.
- **Move the game data folder** (Settings → Storage):
  - **What moves:** versions, libraries, assets, Java runtimes and instances. The database, logs and
    sign-in stay put.
  - **How:** a rename on the same drive, a copy with size checks to another drive. If anything fails,
    everything is put back.
  - **Order:** the new location is saved before old copies are deleted, then the app restarts.
  - **Refused** while games run or downloads are queued, and other changes are blocked during the move.
- **Idle memory, from ~183 MB to ~116 MB** (target < 150 MB, now met):
  - **Breakdown:** the Rust process idles at 11 MB. The largest single item was WebView2's GPU process
    at ~83 MB.
  - **Software rendering by default.** WebView2 now renders in software (`--disable-gpu`), unless the
    new Settings → Hardware acceleration toggle is on.
  - **Measured with and without the GPU:** frame times were identical (p99 5.7 ms on a 180 Hz display,
    no frame over 25 ms) and idle CPU stayed about 0. During constant navigation CPU rose from about
    85% to 95% of one core.
  - **Minimized or hidden for a game:** the WebView gets the low memory-usage target, and the renderer's
    working set drops from ~74 MB to ~22 MB.
- **CI:** a manual end-to-end job (accessibility audit and relocation, with throwaway data folders).

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **149 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` | clean / clean |
| `cargo deny check advisories licenses bans sources` / `pnpm audit` | clean / no known vulnerabilities |
| `pnpm typecheck` / `pnpm test` | clean / **31 passed** |
| Loader network test (`tests/loaders.rs --ignored`, empty folder, 1.7 GB) | **pass**: Fabric 1.21.1 and 26.3, and NeoForge 21.1.265 and 26.3.0.101, whose processors produced patched clients |
| UI smoke test (`e2e/ui-smoke.mjs`) | **19/19 pass** (with software rendering) |
| Accessibility audit (`e2e/a11y-audit.mjs`) | **14/14 pass** |
| Relocation test (`e2e/relocate.mjs`, throwaway APPDATA) | **15/15 pass**: refusals, move out, restart onto the new folder, move back, restart |
| Startup benchmark (release, 5 runs) | UI ready 255–264 ms; private memory **115–120 MB**; idle CPU 0 % |
| Hardware acceleration on (release) | 176 MB, so the setting is applied |

New unit tests cover:
- Relocation: rename and copy paths, rollback after a failed commit, the location file, and target
  validation.
- NeoForge entry extraction limits and traversal.
- Hostile jar metadata.
- The mod update notices and the data-folder target path.

## Bugs found and fixed during this stage

1. **Path comparison** in the relocation target check: a folder that doesn't exist yet stayed as
   `C:\…` while the current folder became `\\?\C:\…`, so "inside the current folder" wasn't detected.
   A unit test caught it, and the nearest existing parent is now canonicalized.
2. **Clippy:** a redundant conversion and an unused variable, both introduced and fixed in this stage.
3. **Test harness:** the first relocation run closed the app before the move was sent. The driver now
   has `detach()` and can attach to a restarted app.

## Decisions to confirm

- **GPU acceleration is off by default.** It's the change that meets the memory target, and this UI
  measured just as smooth without it. If you'd rather keep the GPU on by default, flip `gpu_acceleration`'s
  default in `crates/nc-store/src/settings.rs`. The memory target would then be missed (~176 MB).

## Not done or not verified

- **Copying to another drive** was not tested end to end, because this PC has only drive C:. The copy
  path is unit-tested by forcing a copy, including rollback.
- **The low memory target** was measured when minimized, but not while hidden during a game. Launching
  the game is blocked by R1.
- **Software rendering** was measured on one high-end PC at 1180×740. Slow CPUs and 4K displays were
  not measured, which is what the setting is for.
- **Signed updates** are Stage 8.
- **Token copies** inside HTTP and JSON library buffers are outside `zeroize`'s reach (see the security
  review).
- **The CI e2e job and `pnpm audit` step** have not run, because there's no remote. The workflow YAML
  was checked by reading it; there is no YAML parser here.
- **Sign-in, launching, and mods in a real game** are still blocked by R1.

## Next proposed stage: Stage 8, public beta preparation

- Tauri updater with signed artifacts (signing keys kept outside the repository).
- NSIS installer polish, a first-run flow, crash and diagnostic export.
- Release notes, a privacy statement and contributor docs.
- A GitHub remote, so CI finally runs.
- Resolve R1 (Mojang app-ID approval) before any public build.
- Publishing or pushing only with your explicit approval.
