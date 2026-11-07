# Stage 0 report: technical proof of concept

Branch: `feat/stage-0-poc` (local, not pushed) · Date: 2026-11-08

## Summary

The risky parts of the launcher are proven on real infrastructure:

- **Installing:** the official manifest → version JSON → libraries, assets, client and log config →
  official Mojang Java 25 runtime. 26.3 installs in 16 s and every one of 5,223 files is SHA-1 verified.
  A re-run downloads 0 bytes. Killing the process mid-download and resuming leaves no corrupt or partial files.
- **Launch path:** the generated classpath (75 entries) and main class load in the installed runtime.
  Launching the game itself is blocked on Microsoft sign-in (see below).
- **Desktop app:** Tauri 2 + React UI. UI ready in about 450 ms, idle CPU about 0%, no main-thread long
  tasks during a 779 MB install driven from the UI.
- **Sign-in:** the Microsoft → Xbox → XSTS → Minecraft chain, PKCE loopback, and all known error cases
  are implemented and tested against mocks. **Not yet run end to end: Mojang has not approved an app ID.**
- **NeoForge:** installer format analysed. Recommendation: native processor execution
  ([spikes/neoforge.md](spikes/neoforge.md)).

## Acceptance criteria

| Criterion | Status | Evidence |
|---|---|---|
| `cargo test --workspace` and `pnpm test` pass **in CI on windows-latest** | ⚠️ Pass **locally**. The CI workflow is written but has not run (no remote). | 66 Rust + 10 frontend tests, `clippy -D warnings` clean |
| Fresh install of the latest release completes and every file is hash-verified. A second run downloads 0 bytes. | ✅ | benchmarks/stage-0.md |
| Killing the app mid-download and restarting resumes without corruption | ✅ | 3 SIGKILLs at 3–5 s, then resume: 0 `.part` files left, `verify` OK |
| Cancel stops all network activity within 1 s | ✅ (engine) | `cancel_stops_within_one_second` test. The UI Cancel button was not exercised by automation. |
| UI responsive during install (< 100 ms, no long tasks > 50 ms) | ✅ | 0 long tasks, frame latency < 1 ms |
| Game starts, logs stream into the UI, exit code reported | ⚠️ **Partially** | Spawn, log redaction and batching, and the exit event are implemented. Main class and classpath verified without credentials. Reaching the title screen is **blocked on R1** (no approved app ID). Offline launch is deliberately not used. |
| Auth spike reaches XSTS, and the profile if approved. No token in any log. | ⚠️ **Mock only** | 12 auth tests including Debug-redaction checks. No real Microsoft run yet (needs your Azure registration). |
| NeoForge spike report with recommendation | ✅ (static analysis) | The installer profile was inspected but **not executed**. |
| Benchmarks recorded with method and numbers | ✅ | Memory target **missed**: 178 MB private vs < 150 MB |

## Bugs found and fixed during Stage 0

1. A complete `.part` file left by a crash between the last write and the rename triggered
   `416` and a redundant download. It is now verified locally. Regression test added.
2. Fabric profiles carry no hash for `fabric-loader` and `intermediary`. This is recorded as a Stage 4
   requirement (fetch the Maven `.sha1` sidecar). A test pins the behaviour.

## Remaining risks

| # | Risk | Status |
|---|---|---|
| R1 | Minecraft API app-ID approval | **Open, owner action.** Blocks end-to-end sign-in and launch. |
| R2 | Loopback redirect with Entra (`localhost` with any port, IPv6) | Implemented. Unverified against real Entra. |
| R3 | NeoForge processors | Approach chosen. Implementation and timing are Stage 4. |
| R7 | Idle memory 178 MB vs 150 MB target | Open. WebView2 dominates. Mitigations listed for Stage 7. |
| — | Windows 11 snap layouts don't appear on the custom maximize button | Known limitation of custom title bars. Revisit in Stage 6. |
| — | `localhost` app-wide commands: Tauri custom commands are not yet restricted per-window by an app manifest | Low risk with one window. Add `AppManifest` permissions in Stage 1. |
| — | Low-end reference PC not defined | Needed before beta performance claims. |
| — | Legacy (pre-1.7) virtual asset layout | Logged as unsupported. Stage 2. |

## Files

```
Cargo.toml  rust-toolchain.toml  clippy.toml  deny.toml  package.json  pnpm-workspace.yaml
crates/nc-core/        src/{lib,error,secret,paths,maven,manifest,rules,libraries,profile,launch,java}.rs
                       tests/fixtures_test.rs, tests/fixtures/*.json (official metadata only)
crates/nc-launcher/    src/{lib,error,layout,fsutil,download,meta,install,java,auth,process}.rs
                       tests/{download,auth}.rs, examples/poc.rs
apps/desktop/          index.html, vite.config.ts, tsconfig.json, package.json, assets/logo.svg
  src/                 main.tsx, App.tsx, styles/app.css, lib/{ipc,format}.ts, stores/launcher.ts,
                       components/{Button,ProgressBar,TitleBar,LogView}.tsx, *.test.ts(x), assets/fonts/*
  src-tauri/           Cargo.toml, build.rs, tauri.conf.json, capabilities/main.json, icons/*,
                       src/{main,lib,state,commands}.rs
scripts/bench-startup.ps1
.github/               workflows/ci.yml, ISSUE_TEMPLATE/*, dependabot.yml
docs/                  architecture.md, dev-setup.md, stage-0-report.md, benchmarks/stage-0.md,
                       spikes/{auth,neoforge}.md, proposal/00-architecture-and-stage-0.md
README.md  CONTRIBUTING.md  SECURITY.md  CHANGELOG.md  LICENSE (Apache-2.0)  NOTICE
```

## Proposed next stage: Stage 1, application foundation

1. `nc-store`: SQLite (rusqlite, WAL) with versioned migrations and migration tests. Instances and settings tables.
2. App shell: sidebar navigation and routing for the screens in the brief, with empty, loading, error and offline states.
3. Instance model: create, rename and delete, an independent game dir per instance, the default instance migrated from Stage 0.
4. Structured logging to a rotating file with redaction. Diagnostics screen v0.
5. Tauri app permission manifest: restrict commands per window.
6. Push to GitHub and get CI green on `windows-latest` (needs the repository name from you).
