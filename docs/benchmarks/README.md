# Benchmarks

Measured on 2026-11-08. Numbers are from single runs on one machine. They are a baseline, not a guarantee.

## Reference PC A (high-end)

AMD Ryzen 7 9800X3D (8C/16T) · 15 GB RAM · RTX 3060 Ti · NVMe · Windows 11 Enterprise 10.0.26300 ·
WebView2 157.0.4341.42 · home broadband (bandwidth not measured).

A low-end reference PC (4 cores, 8 GB, HDD or SATA SSD) still needs to be chosen. Results there will be worse.

## Desktop app: release build, `scripts/bench-startup.ps1`, 5 runs

| Metric | Target | Result (runs 1–5) | Verdict |
|---|---|---|---|
| Process start → first UI paint (`ui_ready`) | ≈2 s | 436 / 434 / 463 / 476 / 464 ms | ✅ |
| Idle CPU, whole process tree (5 s window, after 10 s idle) | minimal | 0.06 / 0 / 0 / 0 / 0 % | ✅ |
| Idle private memory, whole tree (7 processes: app + WebView2) | < 150 MB | 176–179 MB | ❌ **misses by ~28 MB** |
| Idle working set, whole tree (includes shared pages) | — | 343–349 MB | info |

`ui_ready` is measured from the start of `run()` to the first animation frame after React mounts, and is
reported over IPC. Time spent before `main()` (process creation, DLL loading) is not included.

Memory notes: most of the private memory belongs to WebView2's browser, GPU, renderer, network and
utility processes. Options for Stage 7, each to be measured:
- WebView2 memory-target-level API (`MemoryUsageTargetLevel::Low`) when the window is minimized or the game runs.
- Disabling unused WebView2 features through browser arguments.
- Holding the JS bundle under its current 113 KB gzip.

## UI responsiveness during installation (debug build, DevTools protocol)

1.21.1 was installed from the UI (779 MB, 16 s) while a `PerformanceObserver` recorded main-thread
long tasks.

| Metric | Target | Result |
|---|---|---|
| Long tasks > 50 ms during install | 0 | **0** ✅ |
| `requestAnimationFrame` latency at 26 % progress | < 100 ms | **< 1 ms** ✅ |

## Installer: `poc` CLI, release build

| Scenario | Result |
|---|---|
| Fresh install of 26.3 (Java 25 runtime + 5,223 game files, 586 MiB) | 16.0 s |
| Second run (everything verified by SHA-1, nothing downloaded) | 0.3–0.4 s, 0 bytes |
| Killed (SIGKILL) at 4 s, 3 s and 5 s, then resumed | completed, 0 `.part` files left, 0 retries after the fix below |
| `verify` (real classpath, 75 entries, no credentials) | `MissingRequiredOptionsException [accessToken, version]`: classpath and main class OK |

Bug found and fixed during this run: a `.part` file that was complete when the process died caused a
`416 Range Not Satisfiable` and a redundant re-download. Complete parts are now verified without a
request. A regression test was added (`complete_part_file_is_verified_without_a_request`).

## Bundle

| Asset | Size | gzip |
|---|---|---|
| JS | 350 KB | 113 KB |
| CSS | 15.5 KB | 4.3 KB |
| Fonts (2 × variable woff2) | 141 KB | — |
| `night-client.exe` (release, stripped) | 15.5 MB | — |

---

## Stage 1 re-run (2026-11-08, same PC, release build)

`ui_ready` is now read from the rotating log file, because release builds no longer write to stderr.

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 306 ms | 177.5 | 345.7 | 0 % |
| 2 | 291 ms | 176.9 | 348.1 | 0 % |
| 3 | 298 ms | 175.9 | 345.9 | 0 % |
| 4 | 303 ms | 177.6 | 348.7 | 0.12 % |
| 5 | 298 ms | 178.4 | 346.3 | 0 % |

Startup improved from about 450 ms. This is probably because logs now go through a non-blocking file
writer instead of a redirected stderr pipe, but the cause hasn't been isolated. Memory is unchanged and
still misses the 150 MB target. Bundle: JS 385 KB (122 KB gzip), CSS 23.5 KB (5.7 KB gzip).

---

## Stage 2 re-run (2026-11-08, same PC, release build)

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 275 ms | 175.7 | 346.8 | 0 % |
| 2 | 279 ms | 176.9 | 350.2 | 0 % |
| 3 | 278 ms | 175.4 | 346.5 | 0 % |
| 4 | 283 ms | 175.5 | 344.5 | 0 % |
| 5 | 276 ms | 177.0 | 348.0 | 0 % |

A separate 3-run batch just before this one had an outlier: 218 MB private and 0.35 % CPU. Stage 1's
first run showed a similar spike, which points to a cold WebView2 GPU process. The background download
worker sleeps on a `Notify` when idle, and idle CPU stayed at 0 %.

## Version matrix

```bash
cargo test --release -p nc-launcher --test matrix -- --ignored --nocapture
```

Results are in `docs/stage-2-report.md`. All 8 versions from 1.5.2 to 26.3 passed in 12 s on a warm cache.

---

## Stage 3 re-run (2026-11-08, same PC, release build)

| Batch | UI ready | Private MB | Idle CPU |
|---|---|---|---|
| A (runs 3–5 shown) | 267–295 ms | 176.1–181.6 | 0–0.1 % |
| B (5 runs) | 344, 517, 325, 330, 307 ms | 176.2, 183.0, 204.3, 220.0, 181.8 | 0–0.12 % |

Batch B was noisier than any previous batch, while batch A matched Stage 2. The new background
account check runs after first paint, so it isn't part of `ui_ready`. The variance is probably system
noise, but it hasn't been isolated.

---

## Stage 4 re-run (2026-11-08, same PC, release build)

| Batch | UI ready | Private MB | Idle CPU |
|---|---|---|---|
| A (runs 2–5 shown) | 294–311 ms | 174.4, 207.9, 208.0, 215.8 | 0–0.29 % |
| B (5 runs) | 299, 317, 290, 308, 307 ms | 183.5, 175.2, 176.8, 183.7, 177.4 | 0 % |

One run in batch A had 8 processes instead of 7 (an extra WebView2 utility process). Memory is
unchanged against earlier stages and still above target.

## Loader matrix

```bash
cargo test --release -p nc-launcher --test loaders -- --ignored --nocapture
```

Results are in `docs/stage-4-report.md`.

---

## Stage 5 re-run (2026-11-08, same PC, release build)

| Batch | UI ready | Private MB | Idle CPU |
|---|---|---|---|
| A (runs 2–5 shown) | 292–314 ms | 174.9–182.7 | 0–0.02 % |
| B (5 runs) | 298, 326, 317, 309, 321 ms | 176.0, 206.6, 205.8, 184.8, 183.3 | 0–0.1 % |

Startup is unchanged. The Modrinth client is constructed at startup but makes no network requests until used. Memory
shows the same occasional ~205 MB runs seen since Stage 3.

## Stage 6 re-run (2026-11-08, same PC, release build)

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 273 ms | 176.3 | 354.5 | 0 % |
| 2 | 257 ms | 187.4 | 359.4 | 0 % |
| 3 | 256 ms | 176.9 | 350.8 | 0 % |
| 4 | 254 ms | 183.8 | 352.3 | 0 % |
| 5 | 260 ms | 176.1 | 347.1 | 0 % |

Bundle: JS 476 KB (148 KB gzip), CSS 28 KB (6.5 KB gzip); `night-client.exe` 21.1 MB. Startup did not get
slower with the palette, toasts and artwork, and is slightly faster than the Stage 5 batches (single
runs on a desktop PC, so treat the difference as noise). Memory is unchanged and still misses the
150 MB target.

## Stage 7 re-run (2026-11-08, same PC, release build)

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 259 ms | 115.9 | 328.0 | 0 % |
| 2 | 255 ms | 115.3 | 327.3 | 0 % |
| 3 | 257 ms | 119.5 | 329.5 | 0 % |
| 4 | 264 ms | 119.1 | 330.9 | 0 % |
| 5 | 257 ms | 116.9 | 327.9 | 0 % |

**Idle memory now meets the < 150 MB target.** Breakdown of one idle run (private MB):

| Process | GPU on (Stage 6 default) | GPU off (Stage 7 default) |
|---|---|---|
| `night-client.exe` | 10.8 | 10.7 |
| WebView2 browser | 35.9 | 35.0 |
| WebView2 GPU process | 82.8 | 15.2 |
| WebView2 renderer | 32.6 | 33.2 |
| Network + storage services, crashpad | 21.2 | 21.3 |
| **Total** | **183.3** | **115.4** |

Effect of `--disable-gpu`. Debug build, 12 rounds of navigation, palette open/close and smooth
scrolling, with a `requestAnimationFrame` loop running throughout; two runs each:

| | Frame p50 / p99 / max | Frames > 25 ms | CPU while navigating (one core) | Idle CPU |
|---|---|---|---|---|
| GPU | 5.6 / 5.7 / 5.7–12.5 ms | 0 | 84–86 % | 0–0.5 % |
| Software | 5.6 / 5.7 / 5.8 ms | 0 | 94–96 % | 0 % |

The display runs at about 180 Hz. Other flags saved much less (debug build, against two baseline runs
of 258 and 267 MB): `--enable-low-end-device-mode` −7 to −16 MB, `--disable-gpu-compositing` −23 to
−32 MB, versus −71 to −80 MB for `--disable-gpu`. Setting `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` has no
effect, because Tauri passes its own arguments.

Minimized for 20 s with the low memory-usage target: private memory is unchanged (115.6 MB), and the
renderer's working set drops from 73.8 to 21.6 MB (total working set from 327 to 285 MB).
With Hardware acceleration switched on, the release build idles at 176 MB, confirming the setting is applied.

## Stage 8 re-run (2026-11-08, same PC, release build 0.1.0-beta.1)

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 257 ms | 117.4 | 333.4 | 0 % |
| 2 | 256 ms | 117.7 | 330.8 | 0 % |
| 3 | 253 ms | 120.2 | 333.9 | 0 % |
| 4 | 253 ms | 120.4 | 333.7 | 0 % |
| 5 | 254 ms | 117.8 | 330.9 | 0 % |

`night-client.exe` is 22.8 MB (21.3 MB at Stage 7; the updater plugin accounts for the difference) and the NSIS
installer is 6.2 MB. Silent install takes about 1 s. This build has no update channel, so no update check runs at startup.

## Stage 9 re-run (2026-11-08, same PC, release build)

| Run | UI ready | Private MB | Working set MB | Idle CPU |
|---|---|---|---|---|
| 1 | 305 ms | 122.9 | 338.3 | 0.29 % |
| 2 | 307 ms | 118.4 | 332.2 | 0 % |
| 3 | 257 ms | 120.0 | 333.6 | 0 % |
| 4 | 258 ms | 119.9 | 333.7 | 0 % |
| 5 | 258 ms | 118.2 | 331.4 | 0 % |

`night-client.exe` is 23.9 MB (22.8 MB at Stage 8). Runs 3–5 match Stage 8; the first two were slower
by about 50 ms, the cold-start pattern seen in earlier stages. The registry scan for Java only runs when
Java runtimes are listed, not at startup. Memory still meets the < 150 MB target.
