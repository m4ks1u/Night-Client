# Night Client

A fast, secure, open-source launcher for **Minecraft: Java Edition** on Windows 10/11 (x64).

> **Status: Stage 2, installation engine.** Not ready for players yet.
> See [docs/stage-2-report.md](docs/stage-2-report.md) for what works today.

Night Client is **not affiliated with, endorsed by or associated with Mojang Studios or Microsoft**.
"Minecraft" is a trademark of Mojang Synergies AB. You need a Microsoft account that owns
Minecraft: Java Edition to play. Night Client does not distribute any game files: everything is
downloaded from official Mojang servers and verified against Mojang's published hashes.

## Highlights (planned for the public beta)

- Official Microsoft sign-in (OAuth 2.0 + PKCE in your browser). No passwords, no offline/cracked mode.
- Vanilla, Fabric and NeoForge instances, each with its own game directory.
- Official Mojang Java runtimes installed automatically for each version.
- Modrinth mod discovery with dependency resolution and update checks.
- Verified, resumable, cancellable downloads that never freeze the UI.
- A small native app built on Tauri 2 and Rust.

## Repository layout

| Path | What lives there |
|---|---|
| `crates/nc-core` | Pure Minecraft metadata model: manifests, rules, libraries, launch arguments. No I/O. |
| `crates/nc-launcher` | Services: download engine, installer, Java runtimes, Microsoft auth, game process, logging. |
| `crates/nc-store` | SQLite database with versioned migrations: instances and settings (never secrets). |
| `apps/desktop/src-tauri` | Thin Tauri layer: IPC commands, events, capabilities. |
| `apps/desktop/src` | React + TypeScript UI. |
| `docs/` | Architecture, setup, spikes, benchmarks, stage reports. |

See [docs/architecture.md](docs/architecture.md) for details.

## Building

Requirements: Windows 10/11 x64, Rust (pinned in `rust-toolchain.toml`), Visual Studio 2022 Build
Tools with the C++ workload, Node.js 24, and pnpm (via Corepack).

```bash
corepack enable pnpm
pnpm install
pnpm dev        # run the desktop app with hot reload
cargo test --workspace
pnpm test
```

Full instructions: [docs/dev-setup.md](docs/dev-setup.md).

Microsoft sign-in needs an Azure application ID that Mojang has approved for Minecraft services.
Builds without one still install and verify games, but cannot sign in or launch.
See [docs/spikes/auth.md](docs/spikes/auth.md).

## Contributing and security

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [SECURITY.md](SECURITY.md): please report vulnerabilities privately.

## License

[Apache-2.0](LICENSE). The bundled Geist fonts are licensed under the
[SIL Open Font License 1.1](apps/desktop/src/assets/fonts/OFL-Geist.txt).
