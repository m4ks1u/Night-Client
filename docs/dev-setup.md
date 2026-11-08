# Development setup (Windows 10/11 x64)

## 1. Toolchain

```bash
winget install --id Rustlang.Rustup -e
```
```bash
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
```bash
corepack enable pnpm
```

`rust-toolchain.toml` pins the Rust version, and rustup installs it on the first `cargo` call.
WebView2 ships with Windows 11. On Windows 10 the installer bootstraps it.

> Git for Windows puts its own `link.exe` on `PATH`. Rust finds the MSVC linker through
> the Visual Studio installer, so this normally works. If you see `link: extra operand` errors,
> make sure the MSVC tools come before Git's `usr\bin` on your `PATH`.

## 2. Install and run

```bash
pnpm install
```
```bash
pnpm dev
```

## 3. Tests and checks

```bash
cargo test --workspace
```
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
```bash
pnpm typecheck
```
```bash
pnpm test
```

## 4. Release build (unsigned, no installer)

```bash
pnpm --filter @night-client/desktop tauri build --no-bundle
```

## 5. Proof-of-concept CLI

```bash
cargo run --release -p nc-launcher --example poc -- install 26.3
```
```bash
cargo run --release -p nc-launcher --example poc -- verify 26.3
```

`verify` starts the client's main class with the real classpath but **without credentials**.
Minecraft exits with "Missing required option(s) [accessToken]", which proves that the runtime,
classpath and main class resolve, without launching the game.

## 6. Version matrix (real network, about 1–2 GB)

```bash
cargo test --release -p nc-launcher --test matrix -- --ignored --nocapture
```

`NC_MATRIX_VERSIONS=1.8.9,26.3` limits the set, and `NC_MATRIX_DATA=<dir>` uses another data folder.

Real Modrinth API (search, dependency plan, verified install, hash identification, update check):

```bash
cargo test -p nc-launcher --test modrinth -- --ignored --nocapture
```

Loader matrix (Fabric and NeoForge for 1.21.1 and 26.3; runs NeoForge's installer processors):

```bash
cargo test --release -p nc-launcher --test loaders -- --ignored --nocapture
```

## 7. Microsoft sign-in

Sign-in needs your own Azure app registration and Mojang's approval. See [spikes/auth.md](spikes/auth.md).

```bash
NIGHT_CLIENT_MSA_CLIENT_ID=<your-guid> pnpm dev
```

## 8. Demo accounts for UI testing

With the app closed, this adds two demo accounts. DemoAlex gets a dummy DPAPI token; DemoSteve gets none, so it shows as expired. Neither works for real sign-in.

```bash
cargo run -p nc-launcher --example seed_demo_accounts -- "%APPDATA%/NightClient/night-client.db" "%LOCALAPPDATA%/NightClient/vault"
```

Add `--remove` to delete them again.

## 9. Benchmarks and smoke test

```bash
powershell -ExecutionPolicy Bypass -File scripts/bench-startup.ps1 -Runs 5
```

Debug builds accept `NIGHT_CLIENT_DEVTOOLS_PORT=<port>`, which exposes WebView2 DevTools for
automated UI tests. Release builds never open a debugging port.

A debug build (`cargo build -p night-client`) loads its UI from the Vite dev server at
`localhost:1420`, so keep `pnpm dev` running in `apps/desktop`. Close any running Night Client, then run
from `apps/desktop`:

```bash
node e2e/ui-smoke.mjs ../../target/debug/night-client.exe
```
```bash
node e2e/a11y-audit.mjs ../../target/debug/night-client.exe
```

Both scripts take an optional screenshot folder as a second argument. They use your real data folder,
create a temporary instance and delete it again, and exit non-zero on any failed check. The smoke test
needs vanilla 1.21.1 to be installed already, because it runs a verify job.

The relocation test never touches your data: it points `APPDATA`/`LOCALAPPDATA` at a temporary folder,
moves a small seeded data folder out and back, and checks the app restarts onto each one. At the end it
stops every `night-client.exe` started from the given path.

```bash
node e2e/relocate.mjs ../../target/debug/night-client.exe
```

## 10. Dependency audits

Install once with `cargo install cargo-deny --locked`, then:

```bash
cargo deny check advisories licenses bans sources
```
```bash
pnpm audit
```

## 11. More end-to-end tests (Stage 8)

Both use a throwaway `APPDATA`/`LOCALAPPDATA`; run them from `apps/desktop` with `pnpm dev` running.

```bash
node e2e/welcome.mjs ../../target/debug/night-client.exe
```
```bash
node e2e/crash-report.mjs ../../target/debug/night-client.exe
```

The other scripts skip the first-run welcome automatically (it is recorded as seen in that profile).

### Update flow (install → refuse a bad update → update → uninstall)

This one installs Night Client for your Windows user into a test folder, then uninstalls it. Use a
**throwaway** key that never leaves the test folder, and never your release key.

1. Make a test folder `T`, generate a test key and write two config overrides:
   ```bash
   pnpm tauri signer generate -w T/test.key -p test --ci
   ```
   For each `v` in `0.0.1` and `0.0.2`, `T/conf-v.json` holds
   `{"version":"v","bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":"<contents of T/test.key.pub>","dangerousInsecureTransportProtocol":true}}}`.
   The insecure-transport flag only exists in these test overrides. The shipped config requires HTTPS.
2. With `NIGHT_CLIENT_UPDATE_PUBKEY` set to the test public key,
   `NIGHT_CLIENT_UPDATE_ENDPOINT=http://127.0.0.1:8737/latest.json`, and `TAURI_SIGNING_PRIVATE_KEY` /
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` set to the test key, run
   `pnpm tauri build --debug --config T/conf-v.json` for each version. Copy
   `target/debug/bundle/nsis/*setup.exe*` into `T/v/`.
3. Run the test:
   ```bash
   node e2e/update-flow.mjs T
   ```
   Then rebuild normally, so the debug exe no longer carries the test channel.
