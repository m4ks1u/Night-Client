# Stage 14 report: modpack updates

Branch: `feat/stage-14` (local, branched from `feat/stage-13`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

You asked me to keep going without questions, so I took the first item on the Stage 13 shortlist:
updating an instance to a newer version of its modpack. **Nothing has been pushed, tagged or
published.**

## Summary

- **Modpack card.** An instance made from a modpack shows a Modpack card on its Overview tab: the
  pack's icon, name and version, plus **Check for updates**.
  - When a newer version is available, **Update** asks first. If the new version is for another
    Minecraft version, the dialog says so and suggests backing up worlds.
  - During the update the card shows file and byte progress, with Cancel.
  - Afterwards it shows what happened: how many files were added, replaced, removed and unchanged, and a
    list of the files kept as you had them.
- **Your changes are kept.** At import, Night Client now records every file the pack put in the
  instance, with its SHA-1. An update compares, per file, that hash, the file on disk now and the new
  version's file:
  - Files still as the pack left them are replaced, or removed if the new version dropped them.
  - Files you edited, mods you deleted, and files you added yourself are left alone.
  - Mods you disabled stay disabled, even when they're replaced.
  - A kept file stays recorded with its original hash, so later updates still treat it as yours.
- **All or nothing.** The new version's files are downloaded and verified (size, SHA-1, SHA-512) and its
  overrides extracted into a staging folder (`.nc-update`) inside the instance first. Existing files are
  only touched once everything is ready, so a failed or cancelled update changes nothing.
- **Versions follow the pack.** The instance's Minecraft and loader versions follow the new pack version.
  A version that switches to another loader is refused: make a new instance from it instead.
- **Packs imported from a file can be updated too.** At import, the file is looked up on Modrinth by
  its SHA-512. If Modrinth knows it, the instance is linked to that project. If not, the card says
  updates can't be checked.
- **Database migration v8** adds `instance_modpack` and `instance_modpack_files`. Your real database was
  migrated during the UI smoke run, and `night-client.db.v7.bak` was written first, as for every
  migration.
- **Small addition:** `modpack_preview_modrinth` accepts an optional version id. The e2e test uses it to
  install an older version; a version picker can use it later.

## Decisions I made without asking

- **A mod you deleted stays deleted**, even if the new pack version still contains it. Deleting it was
  your choice. To get it back, reinstall it from Discover.
- **A config file you edited is never merged** with the new version. Yours is kept, and the card lists
  it, so you can compare it by hand.
- **Instances imported before this stage have no record**, so no Modpack card. Only the throwaway
  test instances from Stage 13 were affected.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **180 passed**, 0 failed, 4 ignored (network suites) |
| `cargo fmt` / `cargo clippy --workspace --all-targets -D warnings` | clean / clean |
| `pnpm typecheck` / `pnpm test` | clean / **36 passed** |
| `real_modpack_install` (ignored test, run with `--ignored`) | **passed** again after the overrides change: 51 files, 51 overrides |
| `e2e/modpack-update.mjs` (new, real Modrinth) | **21/21**, checked as a list below |
| `e2e/modpacks.mjs` / `packs.mjs` / `mod-plans.mjs` / `mods-large.mjs` | **25/25** / **26/26** / **21/21** / **8/8** |
| Accessibility / relocation / welcome / crash report / density | **14/14**, **15/15**, **17/17**, **8/8**, **15/15** |
| UI smoke (your real data; migrated it to v8) | **19/19** |

`e2e/modpack-update.mjs` installs Fabulously Optimized 14.0.0, makes user changes (edits a config,
deletes a mod, disables a mod, adds an own file), then:
- **Cancelled update:** reports "cancelled" and changes nothing.
- **Real update to 14.1.2:**
  - 27 files added, 17 replaced, 26 removed, 56 unchanged, 2 kept.
  - The edited config is kept, the deleted mod stays deleted, the disabled mod isn't re-enabled, and
    the own file is untouched.
  - No staging folder is left behind.
  - The instance records 14.1.2, with the pack's Minecraft and loader versions.
- **Afterwards:** a second check says the instance is up to date, and the Overview card shows the pack
  and runs the check from the UI.

The first run of this test failed 2 of 21 checks. It updated 14.1.1 to 14.1.2, which turned out to have
identical files, so the update finished in under 1.5 s, before the test's cancel arrived. The test now
picks the oldest version for the same Minecraft release, and cancels after 150 ms.

New Rust tests cover:
- The update plan: every case in the table in `docs/architecture.md`, case-insensitive paths, and what
  stays tracked.
- Applying staged files, including writing a replaced mod back disabled.
- Scanning disabled files.
- A whole update against a mock server: a version whose new file fails SHA-512 changes nothing; the real
  update keeps the user's edits, disabled mod and own file; a repeat update is a no-op.
- Store: the modpack record and file list are replaced (not merged) and cascade on delete; the
  Minecraft version patch works.

Not rerun: `cargo deny` and `pnpm audit` (no dependency changes), and the release benchmark.

## Not done or not verified

- **Sign-in and launching are still blocked by Mojang's approval (R1)**, so I couldn't start an updated
  pack in the game.
- **No automatic update check.** Checks run only when you press the button, so nothing contacts Modrinth
  in the background.
- **No choice of version.** Updates go to the newest installable release; there's no downgrade or
  version picker in the UI yet.
- **Not tested with a Minecraft version change:** the e2e update stays within Minecraft 26.2. The code
  path is the same, and the dialog warns about worlds.
- **The file-picker import** still can't be driven by the e2e driver. The SHA-512 lookup it now does is
  the same Modrinth call the other tests use, but it was not exercised through the dialog.
- **CI still hasn't run**: no pull request is open.

## Next step

Possible Stage 15 items:
- Turning a resource pack on from the launcher (`options.txt`).
- A version picker for modpacks (install and update), now that the commands accept a version id.
