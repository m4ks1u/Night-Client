// A modpack-sized mods folder (DEBUG build, Vite dev server running), on a
// throwaway APPDATA/LOCALAPPDATA.
//
//   node e2e/mods-large.mjs <path-to-debug-night-client.exe> [count=600] [screenshot-dir]
//
// Writes <count> small placeholder .jar files into a Fabric instance, opens
// its Mods tab and measures: time until the list shows, rows in the DOM,
// main-thread long tasks while scrolling to the end, and that the last mod
// can be reached and toggled. The files are hashed and their SHA-512 sent
// to Modrinth for identification (none match), as for any unknown jar.

import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, countArg = "600", outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const N = Number(countArg);
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-mods-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};

let d;
try {
  d = await launch(exe, { port: 9367, env, outDir });
  const inst = ok(
    await d.inv("create_instance", { input: { name: "Large pack", gameVersion: "1.21.1", loader: "fabric", loaderVersion: "0.17.2", maxMemoryMb: null } }),
    "create",
  );
  const mods = join(env.APPDATA, "NightClient", "instances", inst.dirName, "mods");
  mkdirSync(mods, { recursive: true });
  for (let i = 0; i < N; i++) {
    writeFileSync(join(mods, `placeholder-mod-${String(i).padStart(4, "0")}.jar`), `PK placeholder ${i}`);
  }
  r.check(`${N} jars written`, readdirSync(mods).length === N);

  // First listing hashes and records every file; time the second, as a user
  // reopening the tab would see it.
  ok(await d.inv("list_mods", { instanceId: inst.id }), "first listing");
  await d.reload();
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "Large pack");
  await sleep(200);
  await d.key("Enter");
  r.check("instance page opens", await d.waitFor(`!!document.querySelector('[role=tab]')`, 5000));
  await d.js(`window.__lt = []; new PerformanceObserver(l => window.__lt.push(...l.getEntries().map(e => Math.round(e.duration)))).observe({ type: "longtask", buffered: false }); true`);
  const t0 = Date.now();
  await d.click(/^\s*Mods\s*$/i, "[role=tab]");
  const shown = await d.waitFor(`!!document.querySelector('[aria-label="Installed mods"] li')`, 20000);
  const ms = Date.now() - t0;
  r.check("list shows", shown, `${ms} ms`);
  const rows = await d.js(`document.querySelectorAll('[aria-label="Installed mods"] li').length`);
  console.log(`  rows in DOM: ${rows} of ${N}; shown after ${ms} ms (includes the IPC call)`);

  // Scroll to the end in steps, then toggle the last mod.
  // A page at a time, one frame each, until the end stops moving.
  await d.js(`(async () => { const m = document.getElementById('main'); let last = -1; for (let i = 0; i < 1000 && m.scrollTop !== last; i++) { last = m.scrollTop; m.scrollTop += m.clientHeight; await new Promise(r => requestAnimationFrame(() => r())); } return true })()`);
  await sleep(500);
  const last = `placeholder-mod-${String(N - 1).padStart(4, "0")}`;
  const lastVisible = await d.waitFor(`[...document.querySelectorAll('[aria-label="Installed mods"] li')].some(l => l.innerText.includes(${JSON.stringify(last)}))`, 5000);
  r.check("last mod reachable by scrolling", lastVisible);
  const gaps = await d.js(`(() => { const r = [...document.querySelectorAll('[aria-label="Installed mods"] li')].map(l => l.getBoundingClientRect()).sort((a, b) => a.top - b.top); return r.slice(1).map((b, i) => Math.round(b.top - r[i].bottom)).filter(g => Math.abs(g) > 1) })()`);
  r.check("rows sit edge to edge (no overlap or gap)", gaps.length === 0, JSON.stringify(gaps));
  await d.shot(`mods-large-${N}`);
  const lt = await d.js("window.__lt");
  console.log(`  long tasks (>50 ms) during open + scroll: ${JSON.stringify(lt)}`);
  r.check("no long task over 200 ms while opening and scrolling", lt.every((t) => t <= 200), JSON.stringify(lt));
  const sw = `document.getElementById('mod-${last}.jar')`;
  await d.js(`${sw}.click()`);
  r.check("last mod can be disabled", await d.waitFor(`${sw}?.getAttribute('aria-checked') === 'false'`, 8000));
  const errs = await d.errors();
  r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
  globalThis.__rows = rows;
  d.close();
} catch (e) {
  r.check("script finished", false, e.message);
  d?.close();
} finally {
  await sleep(1500);
  rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
}
process.exit(r.print());
