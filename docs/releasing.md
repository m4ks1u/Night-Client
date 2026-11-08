# Releasing Night Client

Releases are built by GitHub Actions and reach users in two deliberate, manual steps: publishing a
draft release, then publishing the update feed. Nothing is published automatically.

## One-time setup (maintainer)

1. **Updater signing key.** On your own machine, outside the repository:
   ```bash
   pnpm --filter @night-client/desktop tauri signer generate -w ~/.night-client/updater.key
   ```
   Keep the private key and its password in a password manager. **Losing it means installed copies can
   never be updated again**; leaking it lets someone sign updates.
2. **Repository settings** (Settings → Secrets and variables → Actions):
   - **Variable** `UPDATE_PUBKEY`: the contents of `updater.key.pub` (public, compiled into builds).
   - **Variable** `MSA_CLIENT_ID`: the Microsoft app (client) ID approved by Mojang for Minecraft.
   - **Secret** `TAURI_SIGNING_PRIVATE_KEY`: the contents of `updater.key`.
   - **Secret** `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password.
3. **Environment** named `release` (Settings → Environments), with yourself as a required reviewer.
   Both release workflows wait for that approval.
4. **User-Agent contact:** `nc_launcher::user_agent()` names the repository URL
   (https://github.com/m4ks1u/Night-Client), which Modrinth requires as a contact. Update it if
   the repository moves.

## Each release

1. Update `CHANGELOG.md` and write `docs/release-notes/<version>.md` (plain text and Markdown; it is
   shown in the app as plain text).
2. Set the same version in `Cargo.toml` (`workspace.package`), `apps/desktop/package.json` and
   `apps/desktop/src-tauri/tauri.conf.json`. Pre-releases use `-beta.N`.
3. Tag and push the tag: `git tag v<version>` and `git push origin v<version>`.
4. The **Release** workflow runs the tests, builds the signed installer and creates a **draft**
   release with:
   - the installer,
   - its `.sig`,
   - `latest.json`,
   - `SHA256SUMS.txt`.
5. Download the draft's installer and test it: install, sign in, launch a game, uninstall.
6. Publish the draft on GitHub.
7. Run **Publish update feed** (Actions → Publish update feed → tag `v<version>`). Installed copies see
   the update within a day, or at once with Settings → Updates → Check now.

To stop a bad release from spreading, run **Publish update feed** again with the previous good tag.
That release's signed version check (`requireSignedVersion`) prevents anyone from serving an *older*
signed build under a newer version number. Users who already updated keep the bad version until a
fixed one is released.

## Not yet in place

- **Authenticode signing** of the installer (a code-signing certificate). Without it, Windows
  SmartScreen warns on first download.
- **The workflows have not run on GitHub yet:** there is no remote. The asset step was dry-run
  locally against a signed test installer, and the whole update path was tested locally with
  throwaway keys (`e2e/update-flow.mjs`).
