# Stage 10 report: Forge and large mod lists

Branch: `feat/stage-10-forge` (local, branched from `feat/stage-9-backlog`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

This stage adds Forge, the most-requested loader still missing, and closes two smaller backlog items
from Stage 9. **Nothing has been pushed, tagged or published in this stage.**

## Summary

- **Forge for Minecraft 1.17 and newer.**
  - Forge's installers from 1.17 on use the same `install_profile.json` spec 1 as NeoForge. So the
    NeoForge installer now takes a distribution (`Dist`), and only three things differ: the Maven
    location (`maven.minecraftforge.net`), the installer name (`forge-<mc>-<v>-installer.jar`), and the
    version list (`<mc>-<v>` entries on Maven, returned unordered).
  - The processor run is shared, with every Stage 4 safeguard: the installer verified against its
    `.sha1`, libraries verified, Mojang mappings supplied by the launcher instead of downloaded by a
    processor, and output hashes checked.
  - Forge 1.16.5 and older use spec 0 installers and aren't offered. The New instance screen says
    Forge is supported from 1.17.
  - Forge instances use Forge mods from Modrinth.
- **Native Quilt builds.** On a Quilt instance, a Quilt build that was published the same day as the
  newest compatible Fabric build is preferred. An older Quilt build never wins over a newer Fabric one.
- **Large mod lists.**
  - Measured first: with 1,500 mods the Mods tab rendered every row, took 614 ms, and had main-thread
    long tasks of up to 250 ms.
  - Above 100 mods, it now renders only the rows on screen (TanStack Virtual, already used for the game
    log), against the page's own scroll container, with `aria-setsize`/`aria-posinset`.
  - At 1,500 mods: 16 rows in the DOM, shown in 315 ms, no long tasks. Lists of 100 or fewer render
    as before.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **159 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy … -D warnings` | clean / clean |
| `cargo deny check` / `pnpm audit` | clean / no known vulnerabilities |
| `pnpm typecheck` / `pnpm test` | clean / **33 passed** |
| Forge network installs (`NC_LOADERS=forge … --test loaders -- --ignored`) | **pass**: Forge 47.4.29 · 1.20.1 (28.8 s, 93 libraries, patched client 4.6 MB), 52.1.21 · 1.21.1 (18.0 s, 107 libraries, 26.8 MB), 66.1.5 · 26.3 (21.3 s, 111 libraries, 79.1 MB). Processors ran, the launch command was checked, main class found. Scratch folder deleted |
| `e2e/mod-plans.mjs` (real Modrinth) | **19/19**, now including: Forge 1.20.1 lists versions, plans JEI, and 1.16.5 is not offered |
| `e2e/mods-large.mjs` | **8/8** at 60, 600 and 1,500 mods: rows edge to edge, last mod reachable and toggled, no long task over 200 ms |
| UI smoke / accessibility / relocation / welcome / crash report / density | **19/19**, **14/14**, **15/15**, **17/17**, **8/8**, **15/15** |
| Release benchmark (5 runs) | UI ready 257–264 ms; private memory 117–118 MB; idle CPU 0 % |

New unit tests cover:
- Forge install keys and launch ids.
- The version filter: sorting, exact game version, the spec 1 boundary, and year-based versions.
- Forge version listing against a mocked Maven.
- The same-day native Quilt preference.

## Bugs found during this stage

1. **The first mods-large run never opened the instance page.** It clicked a card selector that
   doesn't exist; the script now navigates with the command palette, as the UI smoke test does.
2. **The first virtualized run "couldn't reach" the last mod.** The scroll loop moved a fixed
   distance; it now scrolls until the page stops moving. This was in the test, not the app.

## Not done or not verified

- **Sign-in and launching are still blocked by Mojang's approval (R1).** Forge, like NeoForge, is
  verified up to a complete launch command, but no game has been started.
- **Forge for 1.16.5 and older** isn't supported: those installers use a different (spec 0) format.
- **Virtualized rows leave the DOM**, so browser find (Ctrl+F) and Tab only reach rows near the
  viewport. Screen readers still get the list size.
- **CI still hasn't run.** Stage 9 is pushed, but no pull request is open yet.

## Next step

Push `feat/stage-10-forge` (it contains Stage 9) once you approve, then open a pull request into
`main` so CI runs for the first time.
