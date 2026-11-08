# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Pre-1.0 minor versions may break compatibility.

## [Unreleased]

### Added: Stage 4 (mod loaders, instance folders)
- Fabric instances: Fabric Meta profiles, Maven `.sha1` verification for loader and intermediary jars, loader-version picker.
- NeoForge instances: native installer-processor execution (spec 1), verified installer and libraries, Mojang mappings supplied by the launcher; NeoForge 21.x and 26.x verified.
- Loader choice in New instance; change the loader version of an existing instance.
- Custom game folder per instance (migration v4) with a native folder picker and strict path validation.
- Downloads and the queue use loader-aware install keys and labels.

### Fixed
- Log redaction no longer hides diagnostic error codes (`error_code=…`, `code="auth_…"`).

### Added: Stage 3 (authentication persistence, JVM arguments)
- DPAPI-encrypted token vault in %LOCALAPPDATA% (machine-local); refresh tokens never touch SQLite or logs.
- Persistent, multiple Microsoft accounts with switching and removal (migration v3, metadata only).
- Automatic session renewal with refresh-token rotation, revoked-token cleanup, profile-mismatch protection and a startup check.
- Per-instance JVM arguments: shell-free parsing, allowlist validation on save and at launch.
- Accounts screen and JVM-arguments editor.

### Added: Stage 2 (installation engine hardening)
- Legacy asset layouts: pre-1.6 resources copied into the instance, 1.6–1.7 virtual assets; versions 1.5.2 to 26.3 verified by a real-network matrix test.
- Download queue with cancel and remove, plus a background worker; Downloads screen with persistent history (migration v2).
- Repair report listing exactly which files were missing or damaged; the install marker is invalidated during repairs.
- Launch pre-flight that reports missing files instead of starting a broken game.
- Java detection (managed, JAVA_HOME, PATH, vendor folders, Minecraft Launcher), compatibility rating, per-instance Java with validation.
- Startup cleanup of stale `.part` and `.tmp` files.

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
