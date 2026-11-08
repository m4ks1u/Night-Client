# Stage 8 report: public beta preparation

Branch: `feat/stage-8-beta` (local, branched from `feat/stage-7-hardening`, not pushed) · Date: 2026-11-08 ·
Version: `0.1.0-beta.1`

**Nothing has been published, pushed, tagged or deployed.** The beta is prepared but not released, and
the blockers are listed at the end.

## Summary

- **Signed in-app updates** (`tauri-plugin-updater` 2.14.1):
  - **Key handling:** the public key and feed URL are compiled in from release-workflow variables.
    No key is in the repository, and a build without them registers no updater at all.
  - **Least privilege:** checking and installing are Rust commands, and the WebView has no updater
    permission.
  - **Verification:** every download is verified before it runs. `requireSignedVersion` binds the
    signature to the announced version, so an old release can't be replayed as a new one.
  - **Guards:** installs are refused while a game runs, downloads are queued or data is being moved.
  - **UI:** Settings → Updates shows release notes as plain text, with progress. An optional
    automatic check (on by default, can be turned off) runs 20 s after start and shows a notification.
- **Crash reports:**
  - A panic hook logs the panic and writes a redacted `crash-*.txt` next to the logs (newest 10
    kept). The next start shows a one-time notice.
  - Diagnostics → Export… saves a zip with the summary, the 5 newest logs and the crash reports, every
    line redacted again. It never includes the database, the vault or game files, and nothing is
    uploaded.
- **First-run welcome:**
  - Four steps: what it is, where data goes, what it connects to (with the update toggle), and get
    started.
  - Finish, Skip or Esc records it as seen, and Settings → About can show it again.
  - Settings now follows changes made elsewhere, so a stale draft can't overwrite them.
- **Installer:**
  - NSIS for the current user, no admin rights needed, with the app icon. 6.2 MB.
  - Silent install, launch and uninstall were tested with the real release build. Uninstalling keeps
    the user's game data and accounts.
- **Release process:**
  - **`release.yml`** runs on `v*` tags in a `release` environment where reviewers can be required.
    It tests, builds the signed installer, and creates a **draft** release with `latest.json` and
    `SHA256SUMS.txt`.
  - **`publish-feed.yml`** is manual. It points installed copies at a release only after that release
    has been published.
  - **`docs/releasing.md`** covers keys, settings, steps and rollback.
- **Documents:** `PRIVACY.md` lists every connection and everything stored. Release notes for
  0.1.0-beta.1 are written. SECURITY, CONTRIBUTING, README, architecture and dev-setup are updated.
- **Version** set to `0.1.0-beta.1`.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **151 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy … -D warnings` | clean / clean |
| `cargo deny check …` / `pnpm audit` | clean (including the updater's new crates) / no known vulnerabilities |
| `pnpm typecheck` / `pnpm test` | clean / **31 passed** |
| **Update flow** (`e2e/update-flow.mjs`, debug installers signed with a throwaway key) | **11/11**: see below |
| Release installer: silent install → launch → uninstall | **pass**: registry entry, Start Menu shortcut, version 0.1.0-beta.1 runs, uninstall removes the app and shortcuts, user data kept |
| UI smoke / accessibility / relocation | **19/19**, **14/14**, **15/15** |
| Welcome (`e2e/welcome.mjs`, includes axe on 3 steps) | **17/17** |
| Crash report (`e2e/crash-report.mjs`) | **8/8** |
| Debug build without a channel | `check_for_update` reports `configured: false` (the test key did not leak into normal builds) |
| Release benchmark (5 runs) | UI ready 253–257 ms; private memory 117–120 MB; idle CPU 0 % |

The update-flow test:
1. Installs a signed 0.0.1.
2. Finds 0.0.2 on a local feed, and gets the release notes as plain text.
3. **Refuses an update whose signature doesn't match** ("signature verification failed"). The app
   keeps running and stays on 0.0.1.
4. Installs the correctly signed 0.0.2: the registry shows 0.0.2, and the installer restarts the new
   version.
5. Uninstalls cleanly.

## Bugs found and fixed during this stage

1. **`latest.json` would have been malformed.** `Get-Content -Raw` adds PowerShell properties that
   `ConvertTo-Json` serialized into the notes and signature fields. A local dry run of the release
   step caught it. Both values are now read as plain strings, and the JSON is written without a BOM.
2. **Updater artifacts need the public key in the config** at build time. The first test build failed
   on it, so the release workflow now passes it through `--config`.
3. **The e2e driver never skipped the welcome:** `innerText` follows CSS uppercase, and the regex was
   case-sensitive. This also made one welcome check pass falsely. Both are fixed, and the check now
   tests for any open dialog.
4. **The crash notice was consumed by the run that crashed** in the first test design. The test panic
   now fires after startup, as a real crash would.
5. **`NIGHT_CLIENT_MSA_CLIENT_ID` had no rebuild trigger**, so a cached build could keep a stale
   client ID. It's added to `build.rs`.
6. **Test regex:** the crash test's version regex didn't allow `-beta.1`.

## What you need to do before a public release

1. **R1, Mojang app-ID approval.** Register the Entra app and get it approved for Minecraft. Until
   then, signing in and launching can't work in a release build, so a public beta would not be
   playable.
2. **Create the GitHub repository** and tell me its name. Then:
   - Update the User-Agent contact in `nc_launcher::user_agent()`. It names a placeholder repository,
     and Modrinth requires a real contact.
   - CI and the release workflows can finally run. They have never run.
3. **Generate the release signing key yourself** (`docs/releasing.md`), and add the repository
   variables and secrets and the `release` environment. Keep the private key outside the repository.
4. **Decide on Authenticode code signing.** It needs a certificate, which costs money. Without it,
   SmartScreen warns on first run.
5. **Approve the first push and tag.** I won't push, tag or publish without your explicit go-ahead.

## Not done or not verified

- **Sign-in, launching and mods in a real game** are still untested (R1).
- **The GitHub workflows haven't run.** The YAML was reviewed by reading it (there's no YAML parser
  here), and the asset step was dry-run in Windows PowerShell 5.1, while CI uses PowerShell 7.
- **The diagnostics export** was tested at the library level (contents and redaction). Saving through
  the native dialog wasn't automated.
- **The update installer** was tested with debug builds served over local HTTP, with the
  insecure-transport flag set in a test-only override. HTTPS against GitHub has not run.
- **The updater's own User-Agent** is `tauri-plugin-updater/<version>`, as stated in PRIVACY.md.
- **The throwaway test signing key** remains in this session's scratch folder. It is not in the
  repository or in any normal build.
