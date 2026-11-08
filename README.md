# Night Client

An open-source launcher for **Minecraft: Java Edition** on Windows 10 and 11 (x64), built with Rust, Tauri and React.

> **Pre-release development.** No public release is available yet. Microsoft sign-in requires the project's application registration and approval for Minecraft Services API access. Live authentication and gameplay validation are still pending.

## Features

- Vanilla, Fabric, Quilt, NeoForge and Forge instance installation.
- Separate game folders, saves, settings and content for each instance.
- Modrinth mods, resource packs, shader packs and `.mrpack` modpacks.
- Modpack updates that preserve user changes, with journaled recovery after interrupted updates.
- Managed Java runtimes and configurable per-instance Java and memory settings.
- Verified, resumable downloads with cancellation and repair.
- Browser-based Microsoft authentication with PKCE; refresh tokens encrypted locally with Windows DPAPI.
- Signed application-update support when a release signing key and update channel are configured.
- Dark interface, keyboard shortcuts, reduced-motion settings and virtualized large lists.

See [architecture](docs/architecture.md), [security controls and limitations](docs/security-review.md), and [privacy](PRIVACY.md) for implementation details.

## Run from source

Requirements: Windows 10/11 x64, the Rust toolchain pinned in `rust-toolchain.toml`, Visual Studio 2022 Build Tools with the C++ workload, Node.js 24, pnpm and Microsoft Edge WebView2.

```powershell
git clone https://github.com/m4ks1u/Night-Client.git
cd Night-Client
pnpm install --frozen-lockfile
pnpm dev
```

Keep the terminal open while using the development build. Full setup instructions are in [docs/dev-setup.md](docs/dev-setup.md).

### Microsoft sign-in

For development, set the registered Application (client) ID before starting the launcher:

```powershell
$env:NIGHT_CLIENT_MSA_CLIENT_ID = "YOUR_APPLICATION_CLIENT_ID"
pnpm dev
```

The application ID is a public identifier. Minecraft Services API approval is a separate step from registering the application. See [authentication setup](docs/spikes/auth.md). Users of a future configured release will sign in with their own Microsoft accounts; they will not need to register an application.

## Build and test

```powershell
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm typecheck
pnpm test
pnpm build
```

A release installer requires additional signing and channel configuration. See [the release guide](docs/releasing.md). Current verification results and remaining limitations are documented in [the independent acceptance review](docs/independent-audit-acceptance.md). Local benchmark results and methodology are in [docs/benchmarks](docs/benchmarks/README.md); they are not guarantees for other systems.

## Repository layout

| Path | Purpose |
|---|---|
| `crates/nc-core` | Minecraft metadata, validation, rules and launch arguments; no I/O. |
| `crates/nc-launcher` | Downloads, installation, authentication, loaders, Java and game processes. |
| `crates/nc-store` | SQLite storage and migrations. |
| `apps/desktop/src-tauri` | Desktop commands, events and capabilities. |
| `apps/desktop/src` | React and TypeScript interface. |
| `docs` | Setup, architecture, security, verification and release documentation. |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## License and notice

Night Client is licensed under [Apache-2.0](LICENSE). Bundled Geist fonts use the [SIL Open Font License](apps/desktop/src/assets/fonts/OFL-Geist.txt).

Night Client is not affiliated with, endorsed by or associated with Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Synergies AB. A Microsoft account with a valid Minecraft: Java Edition entitlement is required to play. Game files are obtained from their upstream sources; they are not bundled with this repository.
