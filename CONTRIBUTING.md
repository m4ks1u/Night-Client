# Contributing to Night Client

Thanks for your interest! Night Client is in an early stage, so please open an issue to discuss
larger changes before you start.

## Ground rules

- **No authentication bypasses.** Offline/cracked modes and license circumvention will not be accepted.
- **No Mojang assets or game files** in the repository or the installer.
- **Secrets never reach logs.** Wrap tokens in `nc_core::Secret`, and never derive `Serialize` on types that hold them.
- **Untrusted input is validated.** Paths from metadata go through `nc_core::paths::validate_relative` or
  `nc_launcher::fsutil::safe_join`. IPC input is validated in `commands.rs` before use.
- **No shell execution.** Spawn processes with an argument vector.

## Workflow

1. Branch from `main`: `feat/<topic>`, `fix/<topic>` or `docs/<topic>`.
2. Keep commits small and focused. Write messages in the imperative ("Add resumable downloads").
3. Before you push, run:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   pnpm typecheck
   pnpm test
   ```
   UI changes should also pass the desktop end-to-end scripts in `apps/desktop/e2e` (see
   [docs/dev-setup.md](docs/dev-setup.md)).
4. Open a pull request. CI must pass. An independent review (human or Codex) is required before merge.

## Code layout

- `nc-core` stays free of I/O. If you need the network or disk, the code belongs in `nc-launcher`.
- `src-tauri` stays thin: validate input, call a service, map errors. New commands must be added to `build.rs` (permission manifest), `capabilities/main.json`, `lib.rs` and `src/lib/ipc.ts`.
- Schema changes are new files in `crates/nc-store/migrations/`. Never edit a shipped migration.
- UI code uses the design tokens in `apps/desktop/src/styles/app.css`. Don't hard-code colours.

## Tests

- Rust tests for new behaviour. For network behaviour, use `wiremock` rather than real servers.
- Test fixtures under `crates/nc-core/tests/fixtures` are official metadata JSON. Never add game jars or assets.
