# Night Client — Architecture Proposal & Stage 0 Plan

Status: **APPROVED 2026-11-08** (3-crate start, Apache-2.0). Stage 0 results: see ../stage-0-report.md.
Date: 2026-11-07 · Author: Claude (lead architect) · Reviewer: Codex

---

## 1. Environment audit (reference PC)

| Item | Found | Needed for | Action |
|---|---|---|---|
| Project folder | Empty, **not a git repo** | everything | `git init`, `main` + `feat/stage-0-*` branches |
| OS | Windows 11 Enterprise 10.0.26300 x64 | target | — |
| CPU / RAM / GPU | Ryzen 7 9800X3D, 15 GB RAM, RTX 3060 Ti + Radeon iGPU | benchmarks | Record as "reference PC A" (high-end); we also need a low-end reference |
| Disk | ~1.36 TB free on C: | — | — |
| WebView2 | 157.0.4341.42 | Tauri runtime | OK |
| Node / npm | 24.22.1 / 12.2.1 | frontend | OK. Propose `pnpm` via Corepack (lockfile discipline, speed) |
| git | 2.56.0 | VCS | OK |
| **Rust (rustup/cargo)** | **missing** | Tauri, core crate | Install rustup, stable 1.99.1, `x86_64-pc-windows-msvc` |
| **MSVC Build Tools + Windows SDK** | **missing** (no `cl.exe`; `link.exe` on PATH is Git's coreutils one — will shadow MSVC's if PATH order is wrong) | Rust MSVC linking | Install VS 2022 Build Tools, "Desktop development with C++" workload |
| GitHub CLI | missing | PRs / releases | Optional; install with winget if you want me to open PRs |
| Java | none | not required for building | Launcher will manage its own runtimes |

Installing Rust and MSVC Build Tools changes the system, so I'll wait for your go-ahead (commands in §9).

## 2. Current stack versions (checked today against npm / crates.io / rust-lang.org)

| Layer | Package | Latest stable | Note |
|---|---|---|---|
| Toolchain | Rust | 1.99.1 (2026-10-13) | Pin through `rust-toolchain.toml` |
| Shell | `tauri` crate / `@tauri-apps/cli` / `@tauri-apps/api` | 2.12.5 / 2.12.4 / 2.12.4 | |
| Plugins | `tauri-plugin-updater` / `-deep-link` / `-single-instance` / `plugin-opener` | 2.14.1 / 2.6.3 / 2.5.5 / 2.7.2 | We probably don't need deep-link, see §6 |
| Async / HTTP | `tokio` / `reqwest` | 1.54.1 / 0.13.5 | reqwest with `rustls` and no default features |
| DB | `rusqlite` (bundled) | 0.41.1 | Recommended over `sqlx` 0.9.0 (lighter, no macro build cost; we don't need async SQL) |
| Win APIs | `windows` | 0.62.2 | DPAPI, Job Objects, known folders |
| UI | React / Vite / TypeScript | 19.3.1 / 8.3.6 / 7.0.2 | TS 7 is the native (Go) compiler. Confirm `typescript-eslint` support before committing to it, otherwise pin 6.x. |
| Styling | Tailwind CSS | 4.3.5 | v4 CSS-first `@theme` config fits the design tokens well |
| State | Zustand | 5.1.0 | |
| Server state | `@tanstack/react-query` | 5.107.1 | Proposed addition: cache, retries and pagination for IPC queries |
| Lists | `@tanstack/react-virtual` | 3.14.26 | Proposed addition: virtualized mod and log lists |
| Motion / icons / font | `motion` / `lucide-react` / `geist` | 14.10.1 / 1.70.0 / 1.9.2 | Self-host Geist woff2, no Google Fonts call |
| Tests | Vitest | 5.1.0 | + Testing Library; Rust: built-in + `wiremock`, `tempfile`, `proptest` |

## 3. Repository structure

```
night-client/
├─ Cargo.toml                 # workspace
├─ rust-toolchain.toml
├─ crates/
│  ├─ nc-core/                # pure domain: versions, manifests, rules, arg building (no I/O)
│  ├─ nc-net/                 # download engine: queue, retry, resume, verify, cache
│  ├─ nc-fs/                  # safe paths, atomic writes, archive extraction, disk checks
│  ├─ nc-auth/                # MSA PKCE → XBL → XSTS → MC, token vault trait
│  ├─ nc-java/                # runtime discovery, Mojang runtime install, compat rules
│  ├─ nc-loaders/             # Loader trait; vanilla, fabric, neoforge impls
│  ├─ nc-modrinth/            # API client, cache, dependency resolver
│  ├─ nc-store/               # SQLite, migrations, repositories
│  └─ nc-launcher/            # orchestration facade: install/launch/instance services, events
├─ apps/desktop/
│  ├─ src-tauri/              # thin Tauri layer: commands, capabilities, event bridge, DPAPI vault impl
│  └─ src/                    # React app
├─ docs/  (architecture/, adr/, proposal/, build.md, dev-setup.md)
├─ .github/ (workflows/, ISSUE_TEMPLATE/)
├─ README.md CONTRIBUTING.md SECURITY.md CHANGELOG.md LICENSE
```

Nine crates may look like a lot. I'm proposing the split because it keeps compile-time dependencies honest: `nc-core` has no tokio and no reqwest, which makes it fast to test. If you'd rather start smaller, a single `nc-launcher` crate with internal modules is fine for Stage 0–1, and we split later along the same seams. **My recommendation:** start with 3 crates (`nc-core`, `nc-launcher` [net/fs/auth/java/loaders/modrinth as modules], `nc-store`) and split out modules only when one gets big. That avoids premature abstraction.

## 4. Rust crate boundaries & key interfaces

- **`nc-core` (pure):** serde models for `version_manifest_v2`, version JSON (including `inheritsFrom` merging), asset index, rule evaluation (`os`, `arch`, `features`), argument templating (`${auth_player_name}`…), classpath ordering, Java major requirement lookup. Fully unit-testable from fixtures.
- **Download engine:** `DownloadJob { url, dest, expected: Option<Hash{sha1|sha512}>, size }` → bounded `Semaphore` (default 8, per-host caps), exponential backoff with jitter, per-request + idle timeouts, `CancellationToken`, `.part` temp files in the same directory then `rename` (atomic on NTFS within a volume), Range resume when the server sends `Accept-Ranges`, content-addressed cache (`cache/objects/<sha1>`) reused across instances, pre-flight disk-space check. Progress is **aggregated and throttled to ≤10 Hz** before it crosses IPC.
- **Loaders:** `trait Loader { fn id(); async fn resolve(game, loader_ver) -> VersionProfile; async fn post_install(ctx) }`. Vanilla and Fabric are pure profile merges. NeoForge needs installer processors (see Risks). Forge and Quilt slot in later.
- **Auth:** `trait TokenVault { get/put/delete(account_id) }`, implemented in `src-tauri` with **DPAPI (`CryptProtectData`, user scope) + a file in `%LOCALAPPDATA%`**. Windows Credential Manager limits blobs to ~2.5 KB and MSA refresh tokens can get close to that. Only the refresh token is persisted; Minecraft access tokens (24 h) live in memory.
- **Process:** launch Java directly with `std::process::Command` (argument vector, no shell), assign it to a **Windows Job Object** so crashes and orphans are contained, stream stdout/stderr into a ring buffer plus a log file, and redact tokens before anything is written.
- **Store:** `rusqlite` with WAL. Migrations are embedded SQL files keyed by `PRAGMA user_version` and run in a transaction. A single DB actor thread is reached over a channel, so the UI thread never blocks.

## 5. Frontend architecture

- **Routing:** small hash router or TanStack Router (to be decided in Stage 1; no SSR needed).
- **Data:** every Rust command is wrapped in a typed `ipc.ts` layer. Types are generated from Rust with **`specta` / `tauri-specta`** so the two sides never drift (to evaluate in Stage 0). React Query handles request/response; Zustand holds UI-only state (sidebar, active instance, download overlay).
- **Events:** one `downloads://progress` channel and one `game://log` channel. Logs are batched and appended to a virtualized list.
- **Design system:** `src/design/tokens.css` (Tailwind v4 `@theme`) → primitives (`Button`, `IconButton`, `Field`, `Select`, `Dialog`, `Toast`, `ProgressBar`, `EmptyState`, `Skeleton`) → feature screens. Accessible headless parts come from Radix primitives only where needed (Dialog, Select, Tooltip).
- **Palette direction:** near-black `#0B0B10`, graphite surfaces `#131319 / #1A1A22`, one accent "night violet" (~`#7C6CFF`) plus a cool blue for info. Every colour gets its contrast checked. Motion runs 120–200 ms, ease-out, and turns off under `prefers-reduced-motion`.
- **Title bar:** `decorations: false` with a custom drag region; Windows snap layouts kept through `data-tauri-drag-region` on the maximize area.

## 6. Risks & unknowns

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| R1 | **Minecraft API allow-listing.** `api.minecraftservices.com` rejects unapproved Azure client IDs ("Invalid app registration", 403). Approval goes through the *Minecraft Java App ID review* form at aka.ms/mce-reviewappid. Community reports put it at days to weeks; there's no official SLA. | **Critical / schedule** | You register the Entra app (personal accounts only, public client) and submit the form **now**, in parallel with Stage 0–2. The client ID comes from a build variable and is never hard-coded. Until approval we can test MSA + XBL + XSTS, but not the final Minecraft step. |
| R2 | Callback method. The brief asks for system browser + PKCE. On Windows the cleanest option is an **RFC 8252 loopback redirect** (`http://localhost:<ephemeral>`) on a one-shot listener bound to 127.0.0.1 with `state` validation. Custom URI schemes (deep-link) are easier to hijack. | High | Loopback + PKCE (S256) + `state` + 5-min timeout. Verify that Entra accepts `http://localhost` with any port for the "Mobile and desktop" platform (expected yes; Stage 0 spike). Device-code flow as a fallback. |
| R3 | **NeoForge installation** needs the installer's processors (Java programs that patch and remap the client jar). | High | Stage 0 spike: compare (a) running the official installer headless and (b) running processors natively from `install_profile.json`, as Prism/Modrinth App do. Recommendation depends on the spike. |
| R4 | Java requirements are changing: 1.20.5–1.21.x need **Java 21**, 26.1+ needs **Java 25** (per community sources). | Medium | Use the **`javaVersion` field in each version JSON** plus Mojang's official runtime manifest (`java-runtime/…/all.json`, SHA1-verified per file), rather than a hard-coded table or a third-party JDK vendor. |
| R5 | Rule and argument edge cases (legacy `minecraftArguments`, natives classifiers in pre-1.19 versions, `windows-arm64` rules). | Medium | Fixture tests for representative versions (1.8.9, 1.12.2, 1.16.5, 1.20.1, 1.21.x, 26.x). Stage 0 scope is one modern version only. |
| R6 | Token theft / log leaks. | High | Token types that redact themselves in `Debug`/`Display`, a log-redaction layer, DPAPI at rest, no tokens over IPC except the opaque account id. |
| R7 | Memory target <150 MB including WebView2. WebView2 alone is often 80–120 MB. | Medium | Measure in Stage 0. Lazy-load screens, no heavy charts, image cache limits. Treat the number as a target, not a promise. |
| R8 | Trademark: "Minecraft" use and the impression of affiliation. | Medium | Follow the Minecraft Usage Guidelines, add a "not affiliated" notice, use no Mojang assets in branding. |
| R9 | Windows code signing (SmartScreen) needs a paid certificate or Azure Trusted Signing. Updater signing (minisign) is separate and free. | Medium | Updater keys in GitHub Secrets. Authenticode decision deferred to Stage 8. |
| R10 | Codex concurrent edits. | Low | Codex reviews PRs/branches only. We never share a working tree; I'll work in a separate worktree per stage. |

## 7. License recommendation

**Apache-2.0** (alternative: MIT). The whole proposed dependency set is MIT and/or Apache-2.0, so both are compatible. Apache-2.0 adds an explicit patent grant and a `NOTICE` file, which is a natural place for the Mojang/Microsoft non-affiliation notice. A `cargo-deny` licence check in CI will enforce compatibility. Your call.

## 8. Stage 0 — technical proof of concept

**Goal:** prove the risky parts end-to-end with the thinnest code that we can keep (not throwaway): a Tauri window that installs **one vanilla release** and launches it, plus spikes for auth and NeoForge.

**Scope:**
1. Workspace scaffold: Tauri 2 + React/TS/Vite/Tailwind v4, Rust workspace, `rust-toolchain.toml`, lint/format configs, CI skeleton (build + test on `windows-latest`).
2. `nc-core`: parse `version_manifest_v2` + one version JSON, evaluate rules for windows-x64, build classpath + JVM/game args. Fixture-based unit tests.
3. Download engine v0: bounded concurrency, SHA1 verify, `.part` + rename, retry/backoff, cancel, throttled progress events. Tests against a local `wiremock` server: 404, truncated body, hash mismatch, timeout, cancel.
4. Install the latest release into `%APPDATA%\NightClient\instances\poc\` (client jar, libraries, natives, assets) using the official Mojang Java runtime for that version.
5. Launch: offline/cracked launch is excluded by the brief, and every launch (including demo mode) needs a real Minecraft access token. So the game reaches the title screen **only once R1 is approved**. Until then, the launch path is verified by argument and classpath construction tests plus a process-lifecycle test against a dummy Java program. → *Open question for you, see §10.*
6. Auth spike (CLI binary in `nc-auth/examples`): loopback PKCE → MSA → XBL → XSTS → MC → entitlement → profile, redacted logs.
7. NeoForge spike: install one NeoForge version both ways (R3), write up the findings.
8. Measurements: cold start to first paint, idle RSS (process tree incl. WebView2), idle CPU, recorded in `docs/benchmarks/stage-0.md`.

**Acceptance criteria (testable):**
- [ ] `cargo test --workspace` and `pnpm test` pass in CI on `windows-latest`.
- [ ] Fresh install of the latest release completes; every file is hash-verified; a second run downloads 0 bytes (cache hit).
- [ ] Killing the app mid-download and restarting resumes without corrupting the instance (no `.part` left behind and treated as complete).
- [ ] Cancel stops all network activity within 1 s.
- [ ] UI stays responsive during install (input latency <100 ms, no long tasks >50 ms caused by IPC).
- [ ] Game process starts, logs stream into the UI, and exit code is reported. Closing the launcher does not orphan or kill Java unexpectedly (behaviour defined by Job Object flags).
- [ ] Auth spike reaches XSTS (always) and the Minecraft profile (if R1 approved). No token appears in any log file (grep test).
- [ ] NeoForge spike report with a recommendation.
- [ ] Benchmarks recorded with method + numbers; no claims beyond what was measured.

**Out of scope for Stage 0:** final UI and design polish, SQLite, Modrinth, Fabric, multi-instance, updater, installer.

## 9. Setup commands (need your OK — they change the system)

```bash
winget install --id Rustlang.Rustup -e
```
```bash
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
```bash
corepack enable pnpm
```
Optional: `winget install --id GitHub.cli -e`

## 10. Decisions needed from you

1. **Approve** this plan (or the 3-crate start in §3).
2. **Toolchain install:** may I run the §9 commands, or will you install them yourself?
3. **Azure/Mojang registration (R1):** will you create the Entra app and submit the review form? It must be done by the project owner and should start now.
4. **Stage 0 launch auth:** until the client ID is approved we can't get a Minecraft token. OK to have the Stage 0 launch criterion be "launch succeeds once a real token is available", with the launch path verified via argument-construction tests?
5. **License:** Apache-2.0 (recommended) or MIT?
6. **Git hosting:** GitHub org/repo name, and should I `git init` locally now?
