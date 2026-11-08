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
