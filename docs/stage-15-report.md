# Stage 15 report: turning packs on from the launcher

Branch: `feat/stage-15` (local, branched from `feat/stage-14`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

You asked me to keep going without questions, so I took the first item on the Stage 14 shortlist:
turning resource packs on from the launcher. I also did the same for shader packs with Iris, and fixed
a Stage 12 bug I found along the way. **Nothing has been pushed, tagged or published.**

## Summary

- **"In game" switch for resource packs.** Each resource pack row has an "In game" switch next to
  Enabled.
  - Turning a pack on adds it to the instance's `options.txt`, on top, which matches what the game does
    when you pick packs one after another. Turning it off removes it.
  - Every other line of `options.txt` stays as it was, byte for byte and in order.
  - Pack ids follow the game version: `file/<name>` from Minecraft 1.13 on, the bare name before.
  - If `options.txt` doesn't exist yet, it is created with the game's data version (read from the
    installed client jar), so the game doesn't treat it as a file from an old version.
- **"In game" switch for shader packs, with Iris.** On a modded instance with Iris installed, the switch
  sets Iris's shader pack in `config/iris.properties`. Only one shader pack can be on at a time.
  - Without Iris, the switches are greyed out, and a note says to install Iris from Discover → Mods.
  - OptiFine isn't handled: it isn't on Modrinth, so Night Client never installs it.
- **Settings follow the pack.** Updating a pack, which changes its file name, keeps it on, in the same
  place in the list. Removing a pack takes it out of the game's settings.
- **Safe edits.**
  - Nothing is written while the game runs, since the game rewrites `options.txt` when it closes.
  - File names get the same checks as every other pack command, and only installed, enabled packs can
    be turned on.
  - Writes go to a temp file, then a rename. Settings files over 1 MiB are refused.

## Fixed along the way

- **Core-shader packs on vanilla instances went to the wrong folder (Stage 12 bug).** On a vanilla
  instance, Modrinth "shader packs" are core-shader packs, which the game loads as resource packs. Stage
  12 put them in `shaderpacks/`, which vanilla Minecraft never reads, so they could never work.
  - They now install into `resourcepacks/`, show in the Resource packs tab, and can be switched on there.
  - The Shaders tab on a vanilla instance explains this.
  - Packs installed the old way stay in `shaderpacks/`. To use them, remove them and install them again.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **186 passed**, 0 failed, 4 ignored (network suites) |
| `cargo fmt` / `cargo clippy --workspace --all-targets -D warnings` | clean / clean (after fixing one needless borrow clippy reported) |
| `pnpm typecheck` / `pnpm test` | clean / **36 passed** (the hint tests were updated) |
| `e2e/pack-activation.mjs` (new, real Modrinth) | **24/24**, checked as a list below |
| `e2e/packs.mjs` / `modpacks.mjs` / `modpack-update.mjs` | **26/26** / **25/25** / **21/21** |
| `e2e/mod-plans.mjs` / `mods-large.mjs` | **21/21** / **8/8** |
| Accessibility / relocation / welcome / crash report / density | **14/14**, **15/15**, **17/17**, **8/8**, **15/15** |
| UI smoke (your real data) | **19/19** |

The `e2e/pack-activation.mjs` checks:
- **Resource packs:** two packs turned on, in order, with the other `options.txt` lines kept; one turned
  off; removing an active pack drops it from the file.
- **Refused:** unsafe names, packs that aren't installed, and mods.
- **Vanilla core shaders:** one installs as a resource pack, nothing goes to `shaderpacks/`, and the
  search shows it as installed.
- **Fabric shaders:**
  - Without Iris, turning a shader pack on is refused, with a note.
  - With Iris (and Sodium) installed from Modrinth, `iris.properties` gets `shaderPack=…` and
    `enableShaders=true`, which reads back as on, then off again.
- **UI:** the "In game" switch works and the hint shows.

On the first UI run the "Not in game" label wrapped onto two lines; I widened the column and reran:
24/24.

New Rust tests cover:
- Pack ids by version: 1.13+, year-based versions, snapshots, 1.12.2 and older.
- `options.txt` edits: CRLF kept, other lines kept, JSON quotes escaped, new file, missing line, broken
  line.
- `iris.properties`: Latin-1 files, `\u` escapes (including emoji surrogate pairs), a leading space and
  a backslash, a missing `enableShaders`, comments kept.
- On disk: turning packs on and off, following an update or removal, the data version from a client
  jar.

Not rerun: `cargo deny` and `pnpm audit` (no dependency changes), and the release benchmark.

## Not done or not verified

- **Not checked in game.** Sign-in and launching are still blocked by Mojang's approval (R1), so I
  couldn't see a pack turned on in the game. The format matches what Minecraft and Iris write, and is
  covered by the tests above.
- **Incompatible packs.** A pack the game considers incompatible (wrong `pack_format`) is dropped by the
  game unless the player confirms it in the game. Night Client doesn't force it with
  `incompatibleResourcePacks`. Packs installed from Modrinth are filtered by game version, so this
  should be rare.
- **No drag-to-reorder.** The last pack turned on has priority.
- **Disabled packs.** A pack disabled with the Enabled switch stays listed in `options.txt`; the game
  skips missing files and cleans the list itself the next time it runs.
- **CI still hasn't run**: no pull request is open.

## Next step

Possible Stage 16 items:
- A version picker for modpacks (install and update); the commands already accept a version id.
- Reordering resource packs.
