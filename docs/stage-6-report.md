# Stage 6 report: premium UI refinement

Branch: `feat/stage-6-ui` (local, branched from `feat/stage-5-modrinth`, not pushed) · Date: 2026-11-08

## Summary

- **Command palette (Ctrl+K):**
  - A modal `<dialog>` with an ARIA combobox and listbox (`aria-activedescendant`), opened from the
    sidebar's Search button or with Ctrl+K.
  - Goes to any screen, and opens or plays any instance. Matching is word-start, in any order
    ("play surv", "1.21").
  - Arrow keys, Enter and Esc work. After a command runs, focus moves to the new page's heading.
- **Keyboard shortcuts:**
  - Ctrl+N creates a new instance and Ctrl+, opens Settings.
  - Ctrl+F focuses the current screen's search field.
  - Alt+← and the mouse back button go back.
  - All shortcuts are listed in Settings → Keyboard shortcuts.
- **Toasts** (bottom right, `aria-live`), for results that happen off-screen:
  - Install or verify finished, with the repaired-file count, and install failed.
  - The game exited with a non-zero code, with a "View game log" action.
  - Errors use `role=alert` and stay 12 s. Others use `role=status` and stay 6 s. Hover or focus pauses
    the timer, and at most 4 are shown.
- **Instance artwork:** every instance gets a deterministic "night sky" (FNV-1a hash of its id seeds a
  small PRNG) with stars, a horizon tinted by loader (vanilla, Fabric, NeoForge) and a monogram. It
  needs no network and no image files, and is used on the Home hero and on instance cards.
- **Instances:** a grid of artwork cards (default) or a compact list. The choice is remembered per
  machine, in `localStorage` with safe fallbacks.
- **Virtualized log view** (`@tanstack/react-virtual`): the game log renders only the visible rows (27
  of 6,010 px in the smoke test). It stays pinned to the bottom while you are at the bottom, and stops
  following when you scroll up.
- **Reduce motion** setting (Settings → Appearance), stored in the settings table with a serde default,
  so older settings rows still load:
  - It switches Motion to `reducedMotion="always"` and adds a CSS class that disables transitions.
  - The OS `prefers-reduced-motion` preference was already honoured and still is.
- **Windows 11 snap layouts:** hovering the custom maximize button for 500 ms calls `show_snap_layouts`.
  - That Rust command sends Win+Z through `SendInput`, which opens the system flyout.
  - It only does this when the window is focused and not maximized, and it injects nothing else.
- **Accessibility:**
  - A "Skip to content" link. Focus moves to the new page's `<h1>` on navigation.
  - An offline banner (`role=status`).
  - The faint text colour was raised from `#6f6f86` to `#8a8aa0`. It was failing WCAG AA contrast
    (3.3–4.0:1) on 11 of 12 screens and now measures 4.83–5.82:1 on every surface.
- **Title bar:** the badge reads "Beta preview".

## Tests run (all on this machine)

| Suite | Result |
|---|---|
| `cargo test --workspace` | **143 passed**, 0 failed, 3 ignored (network suites) |
| `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` | clean / clean |
| `pnpm typecheck` / `pnpm test` | clean / **29 passed** (5 new) |
| Accessibility audit, axe-core WCAG 2.1 A/AA on 12 screens and dialogs (`e2e/a11y-audit.mjs`) | **14/14 pass** |
| UI smoke test, debug build (`e2e/ui-smoke.mjs`) | **19/19 pass** |
| Startup benchmark (release, 5 runs) | UI ready 254–273 ms; private memory 176–187 MB; idle CPU 0 % |

The audit covers Home, Instances, Discover, Downloads, Settings, Diagnostics, Accounts, New instance, the
instance Overview, Mods and Log tabs, and the delete dialog. It also checks that there are no uncaught
errors and that its test instance is deleted afterwards.

The smoke test covers:
- The palette: opens, filters, runs a command, and moves focus to the new page.
- Log virtualization, the shortcuts and their documentation, and reduce motion (applied and persisted).
- Grid/list persistence across a reload.
- A toast from a real verify job.
- The offline banner in both directions, and the skip link's tab order.
- The snap hover, with no uncaught errors.

New unit tests cover palette matching, artwork determinism, the toast cap and dismissal, the toast alert
role and action, and a regression test for the bug below.

## Bugs found and fixed during this stage

1. **The app rendered blank at startup (React error #185, update loop).** The palette's store selector
   returned a fresh `[]` while instances were loading, which Zustand v5 treats as a change on every
   render. The selector is now stable, and a regression test fails on the old code and passes on the
   new.
2. **Faint text failed WCAG AA contrast** on 11 of 12 screens. The previous token comment claimed AA,
   but that was never measured. Fixed as described above.
3. **E2E harness issues (test-only):**
   - Key events lacked key codes, and pages need focus emulation in the background.
   - The click helper ignored `<label>` names.
   - The scripts didn't check that their cleanup succeeded. Runs that failed midway (during bug 1)
     left two "UI Smoke" instances behind. These were deleted, and both scripts now assert the cleanup.

## Not done or not verified

- **The snap-layout flyout was not seen on screen.** "Snap windows" is turned off on this PC
  (`WindowArrangementActive = 0`), so Win+Z shows nothing. I did not change that system setting. The
  hover path is verified to run without errors, and the window was confirmed to be in the foreground.
- **No density setting.** It was proposed for this stage, but only reduce motion was built.
- **Mod lists are not virtualized.** Only the log is. Mod lists rarely exceed a few hundred rows, and
  this was not measured with a large modpack-sized folder.
- **Memory is still above target**: 176–187 MB private against the 150 MB target, the same as Stage 5.
  The JS bundle is now 476 KB (148 KB gzip; 350 KB at Stage 0). The release exe is 21.1 MB (15.5 MB
  at Stage 0). Sizes weren't recorded in between, so how much of the growth is from this stage is unknown.
- **Screen readers were not tested by hand.** axe checks ARIA structure and contrast, but not
  NVDA/Narrator behaviour.
- **Sign-in, launching and mods loading in a real game** are still unverified (R1). **CI** has still not
  run.

## Next proposed stage: Stage 7, security, testing and optimization

- A security review of IPC inputs, file paths, downloads and log redaction, plus `zeroize` for secrets
  held in memory.
- A dependency audit (`cargo audit`, `pnpm audit`).
- Re-resolve new dependencies when a mod is updated.
- Data-folder relocation.
- Profile and reduce idle memory towards 150 MB.
- Wire the e2e and accessibility scripts into CI as a manual job.
