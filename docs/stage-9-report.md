# Stage 9 report: backlog

Branch: `feat/stage-9-backlog` (local, branched from `feat/stage-8-beta`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

The brief's eight stages were finished in Stage 8. This stage closes items that earlier reports left
open and that don't depend on Mojang's approval (R1). **Nothing has been pushed, tagged or published.**

## Summary

- **Quilt mod loader.**
  - Profiles come from Quilt Meta v3, which uses the same format as Fabric's, so the Fabric installer
    now takes a "flavor" rather than being duplicated.
  - Quilt Meta returns loader versions unordered (its first entry is a 0.20 beta), so lists are now
    sorted newest first.
  - Library hashes come from Maven `.sha1` sidecars, as for Fabric.
  - Quilt runs Fabric mods, so Quilt instances see Quilt and Fabric mods in Discover, install plans,
    updates and local imports. The reverse is refused: Fabric instances don't accept Quilt jars.
- **Database migration v6.** The e2e test found that the `instances` table had
  `CHECK (loader IN ('vanilla','fabric','neoforge'))`, which refused Quilt instances. v6 rebuilds the
  table without the fixed list; the Rust `Loader` type already validates every read and write.
  Dropping the table would cascade to `instance_mods`, so mod rows are kept aside and restored. The
  usual pre-migration backup (`night-client.db.v5.bak`) is written first.
- **Optional dependencies.** An install plan lists the chosen mod's optional dependencies (up to 8).
  Each one is resolved with its own requirements against what is installed or already planned, and
  shown **unticked** under "Optional extras", with any missing or conflicting dependency it would bring.
  Ticking one adds it and its requirements to the install, each project once.
- **Java from the Windows registry.** Vendor installers (Oracle, Adoptium, Microsoft, Azul, BellSoft,
  Corretto, Semeru) record their install folder in HKLM or HKCU, and those folders are now scanned too.
  So Java installed to a custom folder or another drive is found.
  - The scan only reads registry values, and is bounded to 512 keys and depth 4.
  - Only local absolute paths are used. UNC paths are ignored, as for custom Java.
  - Every result goes through the existing inspection.
  - It runs when Java runtimes are listed, not at startup.
- **Compact layout** (Settings → Appearance).
  - Tailwind 4 derives every padding, gap and control height from `--spacing`, so the setting is
    one class on `<html>` that lowers it from 0.25rem to 0.22rem. Text size doesn't change.
  - The smallest controls stay at least 24 px (WCAG 2.2 target size).
  - The switch is now sized in pixels, because its thumb offsets already were.
- **Repository.** The User-Agent, security-report links, crate metadata and `docs/releasing.md` now
  name https://github.com/m4ks1u/Night-Client. The workflows already use `github.repository`.
- **CI.** The manual e2e job now also runs welcome, crash-report, density and mod-plans.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **157 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy … -D warnings` | clean / clean |
| `cargo deny check` / `pnpm audit` | clean / no known vulnerabilities |
| `pnpm typecheck` / `pnpm test` | clean / **33 passed** |
| Quilt network install (`NC_LOADERS=quilt … --test loaders -- --ignored`) | **pass**: Quilt 0.30.2 on 1.21.1 (19.0 s, 81 libraries) and 26.3 (15.2 s, 83 libraries), launch command checked, main class `org.quiltmc.loader.impl.launch.knot.KnotClient`. Ran in a scratch folder, since deleted |
| `e2e/mod-plans.mjs` (real Modrinth) | **16/16**: Sodium Extra offers 3 optional dependencies, unticked; ticking one changes "Install 2 mods" to "Install 3 mods"; each chosen mod installed once; Quilt plans and searches Fabric mods |
| `e2e/density.mjs` | **15/15**: spacing 0.25 → 0.22rem, text 14 px unchanged, Settings page 1704 → 1589 px tall, no control below 24 px on 5 screens, axe WCAG 2.2 AA clean, setting survives a restart |
| UI smoke / accessibility / relocation / welcome / crash report | **19/19**, **14/14**, **15/15**, **17/17**, **8/8** |
| Release benchmark (5 runs) | UI ready 257–307 ms; private memory 118–123 MB; idle CPU 0–0.29 % |

New unit and integration tests cover:
- Quilt install keys and `mod_loaders`.
- Quilt version ordering, against a mock.
- Quilt and Fabric mod compatibility in planning and import.
- Optional dependencies in plans, and merging the install items in the UI.
- The v5 → v6 upgrade keeping every column and mod row, with the cascade still working.
- Registry scanning, against a private HKCU tree that is removed afterwards.
- Path filtering.

## Bugs found and fixed during this stage

1. **Quilt instances couldn't be created.** This was the database CHECK described above, found by the
   new e2e test; fixed by migration v6.
2. **A shell edit damaged two files during development** (`|` used as the Perl delimiter turned
   escaped pipes into regex alternation). It was caught in the diff before any commit, and the files
   were restored and redone with the editor.
3. **The density test flagged the skip link** at 14 px. It is visually hidden until focused, and is
   about 28 px tall when shown, so the measurement now skips unfocused `sr-only` elements.

## Not done or not verified

- **Sign-in, launching, and mods in a real game** are still blocked by R1, Quilt included.
- **Registry detection was tested only against test keys.** This PC has no Java registry entries, so
  real vendor installers weren't exercised.
- **Optional extras only come from the chosen mod.** An optional dependency of a dependency is not
  offered.
- **No preference for native Quilt builds.** When a mod publishes both, the newest compatible file wins
  even if it is the Fabric build.
- **Mod lists are still not virtualized**, and Forge is still not implemented.
- **CI still hasn't run**, because nothing is pushed yet.

## Next step

The repository exists. With your approval I'll push the branches and open the stage work for review
(the plan is in the hand-over message). After CI runs, the remaining blockers are R1, the release signing
key, and the Authenticode decision.
