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
- Since the audit fixes, writes to such paths go through `confined::Confined`: an existing junction or
  symlink below the root is never followed (directory junctions, file symlinks, a folder swapped for a
  junction after the check), temporary files are random and created exclusively, and the destination
  folder is resolved again before each rename. Used by `extract_zip`, modpack downloads and overrides,
  and modpack updates. `atomic_write` (and the game-settings writes) also use random, exclusive
  temporary files. Since the re-review fixes, confined renames never replace an existing file, failure
  cleanup never deletes through a swapped folder (RR-002), and modpack downloads create and check their
  `.part` files through the root. Tested by `confined::tests`, `fsutil`, `tests/update_safety.rs` and
  `tests/rereview_regressions.rs`, including real Windows junctions.
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
- **Libraries embedded in Forge installers** (Stage 11, spec 0): only `maven/<path>` for a library the
  profile lists without a URL, written to a `.part` file inside the libraries folder and renamed only
  after its SHA-1 matches the declared one; a library without a declared hash is refused. Covered by
  `extracts_embedded_libraries_verified_and_skips_processor_outputs`.
- **Mod jars:** inspected read-only, with 256 KiB per metadata file. Nothing is extracted or executed.
- **Modpack overrides** (Stage 13): only entries under `overrides/` and `client-overrides/`, each
  checked with the instance path rules (relative, no `..`, no drive or reserved names, not `.natives`
  or `.nc-update`, also not through an 8.3 short name) and written through `Confined`. Symlinks abort the import; the total is capped at 2 GiB; files are
  written as `.part` and renamed. The index itself is capped at 16 MiB. Tested by
  `overrides_client_wins_and_paths_are_checked` and `reads_and_validates_the_index`.
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
- Since Stage 12, when `LOCALAPPDATA`/`APPDATA` are unset or relative, the vault, logs and data fall
  back to the profile's `AppData` folders, never to the working directory (`user_folders_are_never_relative`).
- Minecraft access tokens live only in memory. Decrypted bytes are wiped on the vault's error path.
- Tested by `dpapi_roundtrip_is_encrypted_at_rest`, `tampered_or_foreign_data_is_rejected` and the
  account tests (rotation, revocation, profile mismatch).

### Untrusted mod metadata and files
- Modrinth ids and slugs are validated before use in URLs. File URLs must be HTTPS on
  `cdn.modrinth.com`. File names must be safe names with the kind's extension: `.jar` for mods, `.zip`
  for resource and shader packs (Stage 12), checked on every command, including toggle and remove.
- Turning packs on (Stage 15) writes only `options.txt` and `config/iris.properties` inside the
  instance. It checks the file name like every pack command and only accepts installed, enabled packs.
  Only the one line or key changes, values are JSON- or properties-escaped, writes are atomic and capped
  at 1 MiB, and nothing is written while the game runs.
- Packs are data for the game or a shader mod; Night Client never opens or extracts them. Shader
  packs contain GLSL that the shader mod compiles on the GPU.
- Downloads are verified by size and SHA-1, then SHA-512.
- Project bodies are never rendered.
- Jar metadata is read without executing anything. Since Stage 7, a mod id is only kept if it looks
  like one: 2–64 safe characters.
- Mods are never loaded or run by the launcher. They are only placed in the instance's `mods` folder
  for the game to load.
- Tested by the Modrinth mock-server tests and `inspects_loader_metadata` (including hostile metadata).
- **Modpack files** (Stage 13) may come from the hosts Modrinth allows in packs: `cdn.modrinth.com`,
  `github.com`, `raw.githubusercontent.com` and `gitlab.com`, over HTTPS, without credentials or a
  port. Every file is verified by size, SHA-1 and SHA-512 from the index; a mismatch deletes the file
  and discards the new instance. Paths are validated when the index is parsed and again before each
  download. The WebView only ever passes a token, never a path or URL.

- **Modpack updates** (Stage 14) only replace or delete files whose SHA-1 still matches what the
  pack installed; anything the user changed or added is left alone. Paths come from the recorded list or
  a newly validated index and are joined with `safe_join`. New files are staged and verified in the game
  folder before any existing file is touched. The update version must belong to the instance's recorded
  Modrinth project, and a version using another loader is refused. Since the re-review fixes, a file
  changed after planning is detected by hash when it is moved and the update is undone (RR-003); the
  database records the update in one transaction with its random id (RR-001), and recovery keeps an
  update only when that id is recorded (RR-004).

## Open risks
- **Write races by a local process** (audit NC-AUDIT-001). `Confined` checks every component and
  resolves the destination folder again just before a rename, but a process running as the same user
  could still swap a folder for a junction in between. Closing that fully needs handle-relative Windows
  APIs. Such a process can already write anywhere the user can. During an update, a file is hashed
  after it has been moved aside, so an edit made before the move is always caught; a program that keeps
  the file open and writes through that handle after the hash still writes into the moved-aside copy,
  which is dropped once the update is recorded.
- **No power-loss guarantee.** Files and the journal are flushed, but folder entries are not (`fsync` on
  directories isn't available on Windows). Crash consistency is tested by aborting the process at every
  step, not by cutting power.
- **Not everything is confined yet.** Game-settings writes (`options.txt`, Iris settings) and NeoForge
  installer outputs still use `safe_join` without `Confined`.
- **Modpack update recovery needs the launcher** (NC-AUDIT-002). An update interrupted by a crash or
  power loss is finished or undone the next time the instance is updated or launched from Night Client.
  Starting the game outside Night Client before that would see a half-updated folder. Undoing never
  overwrites a file that appeared in the meantime: then the old file stays in `.nc-update` and the error
  says where.
- **Modpacks choose their own mods** (Stage 13). The hashes prove the files are the ones the pack
  lists, not that the pack author's choices are safe. A pack from a local file is as trustworthy as its
  source; Modrinth packs reference files Modrinth or the listed hosts serve. Overrides can contain any
  config or resource the game reads, but nothing outside the instance folder.
- **Token copies in libraries:** HTTP and JSON libraries may hold token copies in buffers that
  `Secret` never owns, and zeroing cannot reach those. The OS also keeps a copy of the game's command
  line, which carries the access token as Minecraft requires.
- **NeoForge installer processors** run Java code from the NeoForge installer. Integrity is checked
  against the Maven repository's SHA-1, not against a publisher signature.
- **A user-chosen Java** is executed as given. It must be named `java.exe` or `javaw.exe` and live on a
  local drive.
- **Java found through the registry** (Stage 9) comes from HKLM or HKCU, and HKCU is writable by any
  program the user runs. Such a folder is treated like one on PATH: only local absolute paths are
  accepted, it must pass the same inspection, and it is only used when the user picks it or it is the
  best match. A runtime without a `release` file is asked for `java -version` during the scan.
- **Forge** (Stage 10) runs Forge's installer processors with the same safeguards as NeoForge:
  verified installer and libraries, Mojang mappings supplied by the launcher, output hashes checked.
  Integrity is checked against `maven.minecraftforge.net` SHA-1 sidecars, not a publisher signature.
  Stage 11 extends this to Forge 1.12.2 to 1.16.5. Their embedded Forge jars are trusted as far as the
  verified installer that carries them.
- **Quilt** (Stage 9) libraries are checked against the Maven `.sha1` sidecars on `maven.quiltmc.org`
  and `maven.fabricmc.net`, as for Fabric: integrity in transit, not a publisher signature.
- **Custom game folders may be on network shares.** That is data only, but mods placed there are run
  by the game.
- **Builds are unsigned** until Stage 8, and WebView2 is trusted as provided by Windows.
- **Not externally reviewed.** This review was done by the implementer, and CI has never run.
