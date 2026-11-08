# Stage 13 report: Modrinth modpacks

Branch: `feat/stage-13` (local, branched from `feat/stage-12`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

You asked me to keep going without questions, so I took the first item on the Stage 12 shortlist:
Modrinth modpack (`.mrpack`) import. **Nothing has been pushed, tagged or published.**

## Summary

- **Modpacks from Modrinth.** Discover has a fourth content type, Modpacks. Browsing modpacks needs no
  instance, since each pack becomes a new one.
  - Install opens a preview: loader and Minecraft version, pack version, file count, download size,
    and any server-only files left out. You can rename the instance before it is created.
  - The import shows file and byte progress and can be cancelled. When it finishes you land on the new
    instance's Mods tab, and the game and loader are queued for download as for any new instance.
- **Modpacks from a file.** "Import .mrpack file" (Discover → Modpacks) and "Import modpack" (New
  instance screen) open the native file picker. The path stays in Rust; the UI only gets a token.
- **Verified like everything else.**
  - The pack's index is checked in full before anything is written:
    - Format and game.
    - Exactly one supported loader, or none (vanilla).
    - Limits: at most 4000 files and 8 GiB.
    - File paths: relative, unique, and never under `.natives`.
    - SHA-1 and SHA-512 for every file.
    - Download URLs: HTTPS only, on the hosts Modrinth allows in packs (`cdn.modrinth.com`,
      `github.com`, `raw.githubusercontent.com`, `gitlab.com`).
  - Every file is checked by size, SHA-1 and SHA-512.
  - Overrides are copied with the same path rules, no symlinks, and a 2 GiB cap. `client-overrides/`
    wins over `overrides/`.
- **Nothing half-made.** If a download fails, a hash doesn't match or you cancel, the new instance is
  removed again: its row is deleted, and its folder goes to the data folder's `trash`.
- **No database migration.** Modpack mods are ordinary files in `mods/` and are identified on Modrinth
  by hash, like any mod, the next time the list loads.

## Decisions I made without asking

- **Newest release by default.** When you install from Modrinth, I pick the newest release built for a
  supported loader, falling back to the newest beta. There is no version picker yet.
- **Optional files are installed.** Optional files in a pack are installed, and you can disable them
  in the Mods tab. The preview says how many there are.
- **Downloaded packs stay cached.** A pack downloaded from Modrinth is kept in
  `cache/modpacks/<sha1>.mrpack`, so a second import of the same version doesn't download it again.
- **Not every hit is installable.** Modpack search lists packs for Fabric, Quilt, NeoForge and Forge.
  A pack for a Forge version Night Client doesn't support (older than 1.12.2) is refused at preview,
  with a message.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **175 passed**, 0 failed, 4 ignored (network suites) |
| `cargo fmt` / `cargo clippy --workspace --all-targets -D warnings` | clean / clean |
| `pnpm typecheck` / `pnpm test` | clean / **36 passed** |
| Real-network Rust test `real_modpack_install` (run with `--ignored`) | **passed**: the 8 most-downloaded packs all fetched, verified and validated (Fabric and Forge, Minecraft 1.20.1 to 26.3). The smallest, Fabulously Optimized, was fully installed: 51 files and 51 overrides |
| `e2e/modpacks.mjs` (new, real Modrinth) | **25/25**, checked as a list below |
| `e2e/packs.mjs` / `mod-plans.mjs` / `mods-large.mjs` | **26/26** / **21/21** / **8/8** |
| Accessibility / relocation / welcome / crash report / density | **14/14**, **15/15**, **17/17**, **8/8**, **15/15** |
| UI smoke (your real data) | **19/19** |

The `e2e/modpacks.mjs` checks:
- Search works, and bad input is refused: an invalid id, a mod that isn't a modpack, an unknown token, a
  token reused.
- The preview is correct.
- A cancelled import reports "cancelled" and leaves no instance and no folder.
- The UI flow works: the palette opens Discover on Modpacks, search, Install, the dialog, progress, and
  you land on the Mods tab with 49 mods listed.
- On disk:
  - All 49 jars are in `mods/`.
  - The overrides (`config/`, `resourcepacks/`) were copied, and the index itself was not.
  - All 49 mods are identified on Modrinth.
- No UI errors.

New Rust tests cover:
- Index validation: hosts, paths, hashes, loaders and limits.
- Override extraction: client overrides win; traversal, `.natives` and drive paths are refused; the size
  cap holds.
- The modpack search facets against a mock server.
- Version picking.
- SHA-512 verification of pack files: a mismatch is refused and the file deleted.

Not rerun: `cargo deny` and `pnpm audit`. The only dependency change is `url`, already in the
workspace, added to nc-core. The release benchmark was not rerun either.

## Not done or not verified

- **Sign-in and launching are still blocked by Mojang's approval (R1).** I couldn't start an imported
  pack in the game. The files are where the loader reads them, and the instance has the pack's exact
  loader version.
- **No modpack updates.** The instance doesn't remember which pack and version it came from. Updating
  would need a migration to store that, and a careful merge with your own changes.
- **No version picker** for modpacks, and no CurseForge packs (out of scope for the beta).
- **Pack memory settings and server lists** aren't applied. `.mrpack` has no standard field for memory;
  `servers.dat` arrives only if the pack ships it in its overrides.
- **The file-picker path** (Import .mrpack file) uses the same preview and import code as the Modrinth
  path, but the native dialog can't be driven by the e2e driver. The test checks only that the button is
  there; picking a real file through the dialog was not exercised.
- **CI still hasn't run**: no pull request is open.

## Next step

Possible Stage 14 items:
- Modpack updates: remember the pack's project and version, then offer an update that keeps your own
  changes.
- Turning a resource pack on from the launcher (`options.txt`).
