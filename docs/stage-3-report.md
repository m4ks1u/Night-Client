# Stage 3 report: authentication persistence and Java arguments

Branch: `feat/stage-3-auth-java` (local, branched from `feat/stage-2-install-engine`, not pushed) · Date: 2026-11-08

## Summary

- **DPAPI token vault** (`nc_launcher::vault`): Microsoft refresh tokens are encrypted with
  `CryptProtectData` (current user, UI forbidden, app-specific entropy) and written atomically to
  `%LOCALAPPDATA%\NightClient\vault\<uuid>.ncv`. That folder is machine-local and never roamed. Keys are
  validated profile UUIDs. Tampered or foreign files are rejected by DPAPI's integrity check. Output
  buffers are zeroed before `LocalFree`.
- **Persistent, multiple accounts:**
  - Migration v3 adds an `accounts` table holding name, UUID, last used and active flag only. A partial
    unique index allows at most one active account.
  - Sign-in persists the account, which survives restarts. You can add several and switch between them
    (switching is blocked while a game runs). Removing an account deletes its vault entry and promotes the
    next account.
- **Session renewal** (`nc_launcher::accounts`):
  - Minecraft access tokens stay in memory and are renewed from the vault when missing or within 5 minutes
    of expiry. Renewal re-runs the full Xbox → XSTS → Minecraft → entitlement → profile chain.
  - Renewals are serialised so a rotating refresh token can't be raced. The rotated token is saved
    *before* the rest of the chain, so a later failure can't lose it.
  - A revoked token (`invalid_grant`) is deleted and the UI asks you to sign in again. A token that
    returns a different profile is refused.
  - The active account is checked in the background at startup, so problems show before you press Play.
- **JVM arguments** (`nc_core::jvmargs`):
  - Arguments are split without a shell (quotes supported, no expansion) and checked against an allowlist:
    `-XX:…`, `-D…`, `-Xss`, `-Xmn`, `-Xlog`, `--add-opens/-exports/-reads/-modules`,
    `--enable-native-access`, and `-ea`/`-da`.
  - Refused: agents, classpath and module-path changes, `-Xbootclasspath`, `-XX:OnError` and
    `-XX:OnOutOfMemoryError` (these run commands), `-XX:Flags`/`VMOptionsFile` and `@argfiles` (these
    load more options), and launcher-owned properties such as native-library paths and the log4j config.
    `-Xmx`/`-Xms` point you to the memory slider.
  - Arguments are validated on save and again at launch, in case the database was edited.
- **UI:** a multi-account Accounts screen (active, expired, "Sign in again", remove with confirmation),
  an account chip in the sidebar showing state, and a JVM-arguments editor in instance details.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **120 passed**, 0 failed, 1 ignored (network version matrix) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `pnpm typecheck` / `pnpm test` | clean / **23 passed** |
| Desktop smoke test (debug build, 19 checks, seeded demo accounts) | **19/19 pass** |
| Startup benchmark (release, two 5-run batches) | UI ready 267–295 ms, then 307–517 ms; private memory 176–182 MB, then 176–220 MB; idle CPU ≤ 0.12 % |

New tests:
- **Vault:** real DPAPI round-trip, no plaintext on disk, tampering detected, keys validated.
- **Session renewal** (mock Microsoft/Xbox/Minecraft): rotation stored, revoked token deleted, no
  network call when no token exists, profile mismatch refused, rotated token kept when the chain later fails.
- **JVM arguments:** shell-free splitting, 18 dangerous forms blocked, malformed input rejected.
- **Account store:** single active account, switch, remove-and-promote, input validation.
- **JVM-argument storage bounds**, and the frontend quoting round-trip.

Smoke-test highlights:
- Accounts seeded before launch appeared after a restart, and **no token material crossed IPC**.
- Renewal without a configured client failed cleanly.
- The expired account showed "Sign in again", and a launch attempt returned an auth error with no
  process started.
- `-javaagent` was refused in the UI and `-XX:OnError=calc.exe` over raw IPC.
- Quoted arguments round-tripped through the editor.
- Removing the active account promoted the other; removing the last one deleted its vault file.

The seeded accounts come from `cargo run -p nc-launcher --example seed_demo_accounts`, a developer tool
that writes demo metadata and a dummy, non-working token. They were removed afterwards, along with all
smoke-test instances.

## Not done or not verified

- **Real Microsoft sign-in, token renewal and game launch are still unverified end to end.** This is
  blocked on R1 (Azure app registration and Mojang's approval). Everything up to the HTTP boundary is
  tested against mocks, and the vault is tested for real.
- **CI still not run** (no GitHub remote).
- `Secret` values aren't zeroised in memory when dropped (DPAPI buffers are). Adding the `zeroize`
  crate is a candidate for Stage 7.
- Player heads and skins aren't shown. That needs an image CDN in the CSP; deferred to Stage 6.
- Benchmark variance was higher in the second batch (up to 517 ms and 220 MB). That looks like system
  noise, since the first batch matched Stage 2, but it hasn't been isolated. The memory target is still missed.

## Next proposed stage: Stage 4, instances and modloaders

1. Fabric: profile from Fabric Meta, the loader and intermediary hash taken from Maven `.sha1` sidecars,
   and loader version selection.
2. NeoForge: native processor execution (the option recommended in the spike), gated on
   `install_profile` spec 1, with a CI cross-check against the official installer.
3. Loader choice in New instance, plus changing the loader version of an existing instance.
4. A configurable game directory per instance and a data-folder location setting (the roaming-profile risk).
