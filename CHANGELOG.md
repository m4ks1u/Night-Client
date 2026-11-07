# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Pre-1.0 minor versions may break compatibility.

## [Unreleased]

### Added — Stage 0 (technical proof of concept)
- Rust workspace with `nc-core` (pure metadata and launch-argument model) and `nc-launcher` (services).
- Parsing of Mojang version manifests and version JSON, platform rules, legacy natives, `inheritsFrom` merging (Fabric profile verified).
- Download engine: bounded concurrency, SHA-1 verification, `.part` + atomic rename, HTTP Range resume, retry with backoff, timeouts, cancellation, disk-space check, HTTPS-only.
- Vanilla installer (client, libraries, assets, log config) and official Mojang Java runtime installer.
- Microsoft → Xbox Live → XSTS → Minecraft sign-in chain with PKCE loopback callback (pending Mojang app-ID approval for end-to-end use).
- Game process spawning without a shell, with redacted, batched log streaming.
- Tauri 2 desktop shell with a custom title bar, design tokens, and a proof-of-concept screen.
- Benchmarks, NeoForge and auth spike reports, CI workflow.
