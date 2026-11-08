# Security review

Reviewed 2026-11-08 (Stage 7). For each security requirement in the brief: how it is met, where, and what
tests it. "Open" lists what is not done or relies on something outside Night Client.

## Requirements

### Least-privilege Tauri capabilities
- `capabilities/main.json` grants the `main` window its own app commands (each needs an `allow-*`
  entry, generated from `build.rs`) and seven core permissions: window minimize, toggle maximize,
  is-maximized, close, start-dragging, and event listen/unlisten.
- No file system, shell, HTTP, opener or dialog permission is granted to the WebView. The opener and
  dialog plugins are only called from Rust.
- Stage 7 narrowed `core:event:default` to listen/unlisten, because the WebView never emits events.
- CSP: `default-src 'self'`, scripts from `'self'` only, images limited to `'self'`, `data:`, `asset:`
  and `cdn.modrinth.com`. `connect-src` is IPC only. `withGlobalTauri` is off and `freezePrototype`
  is on.

### Strict IPC input validation
Every command validates its input in Rust before acting:
- Instance ids are 36 hex/hyphen characters, and account UUIDs are 32 hex characters.
- Install keys are parsed. Version ids, Modrinth ids and mod file names are each checked against a
  pattern.
- Settings ranges are checked in `AppSettings::validate`. JVM arguments are checked against an
  allowlist on save and again at launch.
- Paths: Java must be absolute and local (UNC and device paths refused since Stage 7). Game folders and
  data-folder targets have their own validators.

Tested by the unit tests in `commands/mod.rs`, `jvmargs.rs`, `paths.rs` and `maven.rs`, by the
refusals in `e2e/relocate.mjs`, and by the raw-IPC refusal checks run in Stage 5.

### Safe process execution, no shell injection
- Games, `java -version` probes and NeoForge processors are started with `Command` and an argument
  vector. Nothing goes through a shell.
- `JAVA_TOOL_OPTIONS`, `_JAVA_OPTIONS` and `JDK_JAVA_OPTIONS` are removed from the game's environment.
- JVM arguments that load code or run commands (`-javaagent`, `-XX:OnError`, …) are rejected.
- Tested by `jvmargs::rejects_code_loading_and_command_execution` and
  `javadetect::rejects_non_java_paths`.

### Safe file-system access and path traversal
- Every name that becomes a path segment passes `nc_core::paths::validate_segment`: no `..`, no
  separators, no drive letters, no reserved device names, no alternate data streams.
- Relative paths from metadata go through `fsutil::safe_join`.
- Maven coordinates are checked segment by segment.
- Tested by `paths::rejects_traversal_and_windows_tricks`, `layout::rejects_traversal_in_ids`,
  `maven::rejects_malicious_coordinates`, `library_with_traversal_path_is_rejected`,
  `java::plan_rejects_escaping_paths` and `instance::trash_moves_folder_and_rejects_traversal`.

### Secure archive extraction
- **`fsutil::extract_zip`** (natives): every entry name is validated. Symlink entries abort the
  extraction, and the total size is capped against zip bombs. Files are written to a temp file, then
  renamed.
- **NeoForge installer entries:** only the named entries are extracted, through `safe_join`. Since
  Stage 7 a 512 MiB total cap fails with an error instead of silently truncating.
- **Mod jars:** inspected read-only, with 256 KiB per metadata file. Nothing is extracted or executed.
- Tested by `extract_rejects_zip_slip`, `extract_enforces_size_cap`,
  `extracts_named_entries_and_fails_instead_of_truncating` and
  `refuses_entry_names_that_escape_the_destination`.

### Dependency vulnerability checks
- `cargo deny check advisories licenses bans sources`: clean on 2026-11-08 (duplicate-version warnings
  only).
- `pnpm audit`: no known vulnerabilities on 2026-11-08.
- Both run in the CI `audit` job.

### Signed application updates
**Open, Stage 8.** No updater exists yet, and nothing is downloaded and run as a launcher update.

### Secret redaction
- Tokens are `nc_core::Secret`: `Debug`/`Display` print `<redacted>`, and the buffer is zeroed on drop
  (Stage 7).
- `LaunchCommand::redacted_args` is used for logging. Game output is redacted against the session's
  secrets before it reaches the UI or the log. The file logger redacts known token shapes.
- Tested by `secret::debug_and_display_never_leak`, `redacts_known_secret_shapes`,
  `writer_redacts_and_tail_reads_last_lines` and `happy_path_yields_profile_and_redacted_session`.

### Secure authentication storage
- Microsoft refresh tokens are DPAPI-encrypted per Windows user in `%LOCALAPPDATA%\NightClient\vault`
  (machine-local, never roamed). SQLite holds account metadata only.
- Minecraft access tokens live only in memory. Decrypted bytes are wiped on the vault's error path.
- Tested by `dpapi_roundtrip_is_encrypted_at_rest`, `tampered_or_foreign_data_is_rejected` and the
  account tests (rotation, revocation, profile mismatch).

### Untrusted mod metadata and files
- Modrinth ids and slugs are validated before use in URLs. File URLs must be HTTPS on
  `cdn.modrinth.com`. File names must be safe `.jar` names.
- Downloads are verified by size and SHA-1, then SHA-512.
- Project bodies are never rendered.
- Jar metadata is read without executing anything. Since Stage 7, a mod id is only kept if it looks
  like one: 2–64 safe characters.
- Mods are never loaded or run by the launcher. They are only placed in the instance's `mods` folder
  for the game to load.
- Tested by the Modrinth mock-server tests and `inspects_loader_metadata` (including hostile metadata).

## Open risks
- **Token copies in libraries:** HTTP and JSON libraries may hold token copies in buffers that
  `Secret` never owns, and zeroing cannot reach those. The OS also keeps a copy of the game's command
  line, which carries the access token as Minecraft requires.
- **NeoForge installer processors** run Java code from the NeoForge installer. Integrity is checked
  against the Maven repository's SHA-1, not against a publisher signature.
- **A user-chosen Java** is executed as given. It must be named `java.exe` or `javaw.exe` and live on a
  local drive.
- **Custom game folders may be on network shares.** That is data only, but mods placed there are run
  by the game.
- **Builds are unsigned** until Stage 8, and WebView2 is trusted as provided by Windows.
- **Not externally reviewed.** This review was done by the implementer, and CI has never run.
