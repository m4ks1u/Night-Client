# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Pre-1.0 minor versions may break compatibility.

## [Unreleased]

### Added: Stage 1 (application foundation)
- `nc-store`: SQLite (WAL) with transactional, versioned migrations, automatic pre-migration backup, and newer-schema protection.
- Instances: create, rename, per-instance memory, open folder, verify files, delete to a recoverable trash folder; play time and last-played tracking.
- App shell with sidebar navigation: Home, Instances, New instance, Instance details (overview, game log), Account, Settings, Diagnostics.
- Settings: default memory, parallel downloads, hide while playing.
- Rotating, non-blocking, redacted file logs in `%LOCALAPPDATA%\NightClient\logs`, with a log viewer in Diagnostics.
- RAM-aware memory recommendations and limits.
- Tauri app permission manifest: each IPC command needs an explicit permission.

### Added — Stage 0 (technical proof of concept)
- Rust workspace with `nc-core` (pure metadata and launch-argument model) and `nc-launcher` (services).
- Parsing of Mojang version manifests and version JSON, platform rules, legacy natives, `inheritsFrom` merging (Fabric profile verified).
- Download engine: bounded concurrency, SHA-1 verification, `.part` + atomic rename, HTTP Range resume, retry with backoff, timeouts, cancellation, disk-space check, HTTPS-only.
- Vanilla installer (client, libraries, assets, log config) and official Mojang Java runtime installer.
- Microsoft → Xbox Live → XSTS → Minecraft sign-in chain with PKCE loopback callback (pending Mojang app-ID approval for end-to-end use).
- Game process spawning without a shell, with redacted, batched log streaming.
- Tauri 2 desktop shell with a custom title bar, design tokens, and a proof-of-concept screen.
- Benchmarks, NeoForge and auth spike reports, CI workflow.
