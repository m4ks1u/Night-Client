// Compact layout (DEBUG build, Vite dev server running), on a throwaway
// APPDATA/LOCALAPPDATA.
//
//   node e2e/density.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// Turns "Compact layout" on in Settings and checks: the spacing scale
// shrinks, text size does not, every interactive control stays at least
// 24 px tall (WCAG 2.2 target size), axe finds nothing, and the setting
// survives a restart.

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const here = dirname(fileURLToPath(import.meta.url));
const AXE = readFileSync(join(here, "..", "node_modules", "axe-core", "axe.min.js"), "utf8");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-density-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };

const measure = `(()=>{
  const px = (v) => parseFloat(v);
  const sw = document.querySelector('[role=switch]');
  const ctl = [...document.querySelectorAll('button,input,select,a[href],[role=switch],[role=radio]')]
    .filter(e => e.offsetParent !== null && !e.closest('[aria-hidden=true]') && !e.matches('.sr-only:not(:focus)'))
    .map(e => ({ h: e.getBoundingClientRect().height, w: e.getBoundingClientRect().width, t: (e.getAttribute('aria-label')||e.textContent||e.id||e.tagName).trim().slice(0,30) }))
    .filter(c => c.h > 0 && c.w > 0);
  return {
    compact: document.documentElement.classList.contains('density-compact'),
    spacing: getComputedStyle(document.documentElement).getPropertyValue('--spacing').trim(),
    body: px(getComputedStyle(document.body).fontSize),
    switchH: sw?.getBoundingClientRect().height,
    small: ctl.filter(c => c.h < 24 && c.w < 24),
    scrollH: document.querySelector('main')?.scrollHeight,
  };
})()`;

let d;
try {
  d = await launch(exe, { port: 9365, env, outDir });
  await d.click(/^\s*Settings\s*$/i, "a,button");
  await d.waitFor(`!!document.querySelector('[role=switch]')`, 8000);
  await sleep(500);
  const before = await d.js(measure);
  r.check("comfortable by default", !before.compact && before.spacing === ".25rem" || before.spacing === "0.25rem", JSON.stringify(before.spacing));
  await d.shot("density-comfortable");

  r.check("toggle present", await d.click(/Compact layout/i, "[role=switch]"));
  await d.waitFor(`document.documentElement.classList.contains('density-compact')`, 4000);
  await sleep(500);
  const after = await d.js(measure);
  r.check("compact class applied", after.compact);
  r.check("spacing scale shrinks", after.spacing === "0.22rem", after.spacing);
  r.check("text size unchanged", after.body === before.body, `${before.body} → ${after.body}`);
  r.check("switch keeps its size", after.switchH === before.switchH, `${before.switchH} → ${after.switchH}`);
  r.check("page gets shorter", after.scrollH < before.scrollH, `${before.scrollH} → ${after.scrollH}`);
  r.check("no control below 24 px", after.small.length === 0, JSON.stringify(after.small.slice(0, 5)));
  await d.shot("density-compact");

  for (const page of ["Home", "Instances", "Downloads", "Diagnostics"]) {
    await d.click(new RegExp(`^\\s*${page}\\s*$`, "i"), "a,button");
    await sleep(700);
    const m = await d.js(measure);
    r.check(`${page}: no control below 24 px`, m.small.length === 0, JSON.stringify(m.small.slice(0, 5)));
  }
  await d.js(`window.axe ? true : (eval(${JSON.stringify(AXE)}), true)`);
  await d.click(/^\s*Settings\s*$/i, "a,button");
  await sleep(500);
  const v = await d.js(`axe.run(document, { runOnly: { type: "tag", values: ["wcag2a","wcag2aa","wcag21a","wcag21aa","wcag22aa"] }, resultTypes: ["violations"] }).then(r => r.violations.map(v => v.id + ":" + v.nodes.length))`);
  r.check("axe (WCAG 2.2 AA) clean in compact Settings", v.length === 0, JSON.stringify(v));
  d.close();
  await sleep(1500);

  d = await launch(exe, { port: 9365, env, outDir });
  r.check("compact survives a restart", await d.waitFor(`document.documentElement.classList.contains('density-compact')`, 8000));
  const errs = await d.errors();
  r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
  d.close();
} catch (e) {
  r.check("script finished", false, e.message);
  d?.close();
} finally {
  await sleep(1500);
  rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
}
process.exit(r.print());
