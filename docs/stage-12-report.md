# Stage 12 report: resource packs and shader packs

Branch: `feat/stage-12` (local, branched from `feat/stage-11`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

You asked me to keep going without questions, so I picked this stage from the Stage 11 shortlist:
Modrinth resource packs and shader packs. **Nothing has been pushed, tagged or published.**

## Summary

- **Resource packs and shader packs from Modrinth.**
  - Discover has a content-type switch: Mods, Resource packs, Shaders. Each type has its own
    categories, and the result badges hide compatibility tags.
  - Every instance gets Resource packs and Shaders tabs, vanilla instances included (they still have
    no Mods tab). The tabs list, enable, disable, remove and update packs, as the Mods tab does for
    mods.
  - Each tab says how to use the files: resource packs are turned on in the game under Options →
    Resource Packs; shader packs need a shader mod such as Iris or OptiFine.
- **One pipeline, not a copy.** A small `ContentKind` model in nc-core carries each kind's folder,
  file extension and accepted Modrinth loader tags. The Modrinth client, version picking, install,
  folder scanning and the Tauri commands all take a kind. Commands default to mods, so existing calls
  are unchanged.
  - Resource packs accept Modrinth's `minecraft` tag.
  - Shader packs on a modded instance accept `iris`, `optifine` and `vanilla` (core-shader) packs.
    On a vanilla instance only core-shader packs are listed, since there is no shader mod.
  - Packs are single files, so there is no dependency resolution. A shader pack's "requires Iris" is a
    mod, which the user installs separately.
- **Same integrity rules as mods.** Files only come from `cdn.modrinth.com` and are verified by size,
  SHA-1 and SHA-512. File names must be safe and end in `.zip` (`.jar` for mods), on every command,
  including toggle and remove. Night Client never opens or extracts a pack.
- **Database migration v7.** `instance_mods` gains a `kind` column, which is now part of the key, so
  the same file name can exist in each folder. Existing rows become mods. Your own database was
  migrated during the UI smoke run, and `night-client.db.v6.bak` was written first, as for every
  migration.

## Found and fixed along the way

1. **Data could land in the working folder.** One e2e run had `APPDATA`/`LOCALAPPDATA` unset. The app
   then created its data folder, logs and token vault relative to the current directory, and
   downloaded a whole Minecraft install (983 MB) into the repository folder.
   - Now these folders fall back to the profile's `AppData\Roaming` / `AppData\Local` (and, as a last
     resort, the temp folder), never a relative path.
   - Verified by starting the rebuilt app from an empty folder with both variables empty: nothing was
     created there, and the app logged to the profile folder. I deleted the stray 983 MB folder.
2. **Screenshots were committed by mistake in Stage 11.** The e2e scripts save screenshots to the
   working folder when no output folder is given, and the Stage 11 docs commit picked up 22 of them.
   They're removed from the repository and now ignored.
3. **The first packs screenshots caught a transition halfway.** The test now checks the pressed
   content type directly and waits for the page to settle before taking a screenshot.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **166 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy … -D warnings` | clean / clean |
| `pnpm typecheck` / `pnpm test` | clean / **36 passed** (3 new for content kinds) |
| `e2e/packs.mjs` (new, real Modrinth) | **26/26**: resource pack search, plan, install, listing, disable/enable, categories; mods still refused on vanilla; vanilla shader search; a `.jar` name refused for packs; shader pack install on Fabric, not listed as a mod; the instance tabs; Discover's switch |
| `e2e/mod-plans.mjs` / `mods-large.mjs` | **21/21** / **8/8** |
| Accessibility / relocation / welcome / crash report / density | **14/14**, **15/15**, **17/17**, **8/8**, **15/15** |
| UI smoke (your real data) | **19/19** on the second run. The first run failed "toast appears when a download job finishes" because it ran with `APPDATA` unset (bug 1 above): the verification downloaded a full game first and exceeded the 60 s wait |

New Rust tests cover:
- Content kinds: folders, extensions and loader tags.
- Pack folder scanning: only `.zip` files, disabled packs, folders and stray `.jar` files skipped.
- The search facets for resource packs and shaders against a mock server.
- Migration v6 to v7, and kind-scoped listing, toggling and removal in the store.
- The data-folder fallback.

Not rerun: `cargo deny` and `pnpm audit` (no dependency changes) and the release benchmark.

## Not done or not verified

- **Sign-in and launching are still blocked by Mojang's approval (R1).** I couldn't check in game that
  a pack loads; the files land in the folders the game and Iris read.
- **Packs aren't turned on automatically.** That would mean editing the game's `options.txt`; for now
  the tab says where to turn them on.
- **No local `.zip` import for packs**, and **no data packs** (they belong to individual worlds).
- **Shader packs on a vanilla instance** list core-shader packs only. Whether a given pack works still
  depends on the pack.
- **CI still hasn't run**: no pull request is open.

## Next step

Possible Stage 13 items: Modrinth modpack (`.mrpack`) import, which now has every content type it
needs; or turning a resource pack on from the launcher (`options.txt`).
