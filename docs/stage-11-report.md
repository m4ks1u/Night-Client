# Stage 11 report: Forge for older Minecraft versions

Branch: `feat/stage-11` (local, branched from `feat/stage-10-forge`, not pushed) · Date: 2026-11-08 ·
Version: still `0.1.0-beta.1`

Stage 10 left Forge for 1.16.5 and older as the main gap. This stage closes most of it. **Nothing has
been pushed, tagged or published.**

## Summary

- **Forge for Minecraft 1.12.2 to 1.16.5.**
  - These installers use `install_profile` spec 0. I compared real installers for 1.12.2, 1.13.2,
    1.14.4, 1.15.2 and 1.16.5 with spec 1: the data, processor and library format is the same, so
    the existing processor code runs them.
  - **Embedded libraries.** The Forge jars themselves have no download URL; they ship inside the
    installer under `maven/`. They are extracted to a `.part` file inside the libraries folder and
    renamed only once their SHA-1 matches the one the profile declares. A library without a declared
    hash is refused, and so is anything over the 512 MiB extraction cap.
  - **Supported range.** Forge for 1.13 and newer, and 1.12.2 from build 14.23.5.2851. Earlier 1.12.2
    builds and older Minecraft versions use the legacy installer format (`install` / `versionInfo`)
    and aren't offered. The New instance text now says Forge is supported from 1.12.2.
- **A real bug found by the installs: Java 8 breaks the output hashes.**
  - The first 1.13.2 install failed: jarsplitter's outputs didn't match Forge's declared SHA-1.
  - I ruled out the time zone (UTC and Europe/Warsaw give the same bytes). Running the same tool on
    each Mojang runtime showed the cause: Java 8u51 deflates differently. Java 17, 21 and 25 all give
    exactly the declared hashes.
  - So spec 0 processors now run on Mojang's Java 17 runtime (`java-runtime-gamma`, installed through
    the same verified runtime installer). The game itself still uses Java 8, as Mojang specifies for
    these versions. 1.12.2 has no processors and downloads no extra runtime.
- **Processor outputs, handled as the official installer does.**
  - A processor whose declared outputs are already valid is skipped.
  - Otherwise, stale outputs are deleted before it runs. Without this, the retry after the Java 8
    failure still failed: jarsplitter's own cache kept the bad file.
- After every Forge or NeoForge install, each runtime library must exist on disk: downloaded, taken
  from the installer, or written by a processor.

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **161 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy … -D warnings` | clean / clean |
| `pnpm typecheck` / `pnpm test` | clean / **33 passed** |
| Forge network installs (`NC_LOADERS=forge … --test loaders -- --ignored`) | **pass**, six versions: 14.23.5.2864 · 1.12.2 (54 libraries), 25.0.223 · 1.13.2 (7.1 s, 62 libraries, patched client 2.9 MB), 36.2.42 · 1.16.5 (13.9 s, 64 libraries, 4.0 MB), 47.4.29 · 1.20.1, 52.1.21 · 1.21.1, 66.1.5 · 26.3. Launch command checked, main class found, no missing jar |
| NeoForge network installs (regression after the output change) | **pass**: 21.1.265 · 1.21.1, 26.3.0.101 · 26.3 |
| `e2e/mod-plans.mjs` (real Modrinth and Forge Maven) | **21/21**, now including: Forge 1.16.5 is offered, 1.12.2 lists only builds 2851 and newer, 1.11.2 isn't offered |
| UI smoke / accessibility / relocation / welcome / crash report / density / large mod list | **19/19**, **14/14**, **15/15**, **17/17**, **8/8**, **15/15**, **8/8** |

New unit tests cover:
- Embedded-library extraction: a verified extract, skipping files a processor will write, no rewrite
  when the file is already valid, and refusal on a wrong hash, a missing hash or the size cap, with
  no file left behind.
- Detection of missing and stale processor outputs.
- The new version filter, both directly and against a mocked Maven.

Not rerun: `cargo deny` and `pnpm audit` (no dependency changes in this stage) and the release
benchmark (no UI or startup changes). The 1.12.2 timing comes from a run where Minecraft 1.12.2 was
already installed, so it isn't shown.

## Not done or not verified

- **Sign-in and launching are still blocked by Mojang's approval (R1).** Old Forge, like the other
  loaders, is verified up to a complete launch command; no game has been started. These versions run
  the game on Mojang's Java 8u51. Some old Forge builds are known to be picky about Java 8 updates, and
  that can only be confirmed by launching.
- **Forge before 1.12.2**, and 1.12.2 builds older than 2851, aren't supported (legacy installer).
- The first Forge 1.13 to 1.16.5 install also downloads the Java 17 runtime (88 MB on disk, measured),
  unless another instance already uses it.
- **CI still hasn't run**: no pull request is open.

## Next step

Possible Stage 12 items: resource packs and shader packs from Modrinth (Discover and per-instance
folders), or Modrinth modpack (`.mrpack`) import.
