// Modrinth modpacks (.mrpack) against the real Modrinth API and CDN (DEBUG
// build, Vite dev server running), on a throwaway APPDATA/LOCALAPPDATA.
// Needs network; downloads one small pack (Fabulously Optimized, ~45 MB).
//
//   node e2e/modpacks.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// 1. Commands: search, refusals (bad id, a mod that isn't a modpack, an
//    unknown token), preview, and a cancelled import that leaves no instance.
// 2. UI: Discover → Modpacks, search, Install, the preview dialog, the
//    import with its progress, then the new instance's Mods tab.
// 3. On disk: every pack file and the overrides are in the instance folder;
//    the mods are identified on Modrinth.
// Nothing is launched.

import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-modpacks-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};
const PACK = "fabulously-optimized";
const instancesDir = join(env.APPDATA, "NightClient", "instances");

let d;
try {
  d = await launch(exe, { port: 9367, env, outDir });

  // 1. Commands.
  const s = ok(await d.inv("modpack_search", { query: "", sort: "downloads", offset: 0 }), "search");
  r.check("modpack search returns results", s.hits.length > 0 && s.total > 0, `${s.total} total`);
  const bad = await d.inv("modpack_preview_modrinth", { projectId: "../x" });
  r.check("an invalid project id is refused", !bad.ok && bad.e?.code === "invalid_input", JSON.stringify(bad.e));
  const mod = await d.inv("modpack_preview_modrinth", { projectId: "sodium" });
  r.check("a mod is not accepted as a modpack", !mod.ok && /isn't a modpack/.test(mod.e?.message ?? ""), JSON.stringify(mod.e));
  const unknown = await d.inv("modpack_import", { token: "pack-999999", name: "x" });
  r.check("an unknown token is refused", !unknown.ok && unknown.e?.code === "not_found");

  const p = ok(await d.inv("modpack_preview_modrinth", { projectId: PACK }), "preview");
  r.check("preview reads the pack", p.loader === "fabric" && p.files > 10 && p.downloadBytes > 1e6 && /^Fabric .+ · /.test(p.label), `${p.name} ${p.label} ${p.files} files`);
  // Start an import and cancel it straight away.
  await d.js(`window.__imp = window.__TAURI_INTERNALS__.invoke("modpack_import", { token: ${JSON.stringify(p.token)}, name: "Cancelled pack" }).then(v=>({ok:true,v}),e=>({ok:false,e})); 1`);
  await sleep(400);
  ok(await d.inv("modpack_cancel", { token: p.token }), "cancel");
  const cancelled = await d.js("window.__imp");
  r.check("cancelled import reports cancelled", !cancelled.ok && cancelled.e?.code === "cancelled", JSON.stringify(cancelled.e ?? "imported"));
  let list = ok(await d.inv("list_instances"), "list");
  r.check("cancelled import leaves no instance", !list.some((i) => i.name === "Cancelled pack"), JSON.stringify(list.map((i) => i.name)));
  const reused = await d.inv("modpack_import", { token: p.token, name: "again" });
  r.check("a token works once", !reused.ok && reused.e?.code === "not_found");

  // 2. UI.
  await d.reload();
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "Discover modpacks");
  await sleep(200);
  await d.key("Enter");
  r.check("palette opens Discover on Modpacks", await d.waitFor(`document.querySelector('[aria-label="Content type"] [aria-pressed=true]')?.textContent.trim() === 'Modpacks'`, 5000));
  r.check("no instance is needed to browse modpacks", await d.waitFor(`document.querySelectorAll('#main ul li').length > 0`, 20000));
  await d.setValue("#main input[type=search]", "Fabulously Optimized");
  r.check("search finds the pack", await d.waitFor(`!!document.querySelector('[aria-label="Install Fabulously Optimized"]')`, 20000));
  await sleep(600);
  await d.shot("modpacks-discover");
  await d.js(`document.querySelector('[aria-label="Install Fabulously Optimized"]').click()`);
  r.check("preview dialog opens", await d.waitFor(`/Install Fabulously Optimized/.test(document.querySelector('dialog[open]')?.innerText||'')`, 60000));
  const dialogText = await d.js(`document.querySelector('dialog[open]').innerText`);
  r.check("dialog shows loader, file count and size", /Fabric .+ · /.test(dialogText) && /\d+ files/.test(dialogText) && /MB to download/.test(dialogText), dialogText.slice(0, 200));
  await d.setValue("dialog[open] input", "FO e2e");
  await sleep(300);
  await d.shot("modpacks-dialog");
  await d.click(/^\s*Create instance\s*$/i, "dialog[open] button");
  r.check("progress is shown while importing", await d.waitFor(`!!document.querySelector('dialog[open] [role=progressbar]')`, 10000));
  r.check("progress counts files", await d.waitFor(`/\\d+ of \\d+ files/.test(document.querySelector('dialog[open]')?.innerText||'')`, 30000));
  await d.shot("modpacks-progress");
  r.check("import finishes on the instance's Mods tab", await d.waitFor(`document.querySelector('[role=tab][aria-selected=true]')?.textContent.trim() === 'Mods'`, 240000));
  r.check("Mods tab lists the pack's mods", await d.waitFor(`document.querySelectorAll('[aria-label="Installed mods"] li').length > 10`, 60000));
  await sleep(800);
  await d.shot("modpacks-instance");

  // 3. On disk.
  list = ok(await d.inv("list_instances"), "list");
  const inst = list.find((i) => i.name === "FO e2e");
  r.check("instance created with the pack's loader", inst?.loader === "fabric" && inst.gameVersion === p.minecraft, JSON.stringify(inst && [inst.loader, inst.gameVersion, inst.loaderVersion]));
  const dir = join(instancesDir, inst.dirName);
  const mods = readdirSync(join(dir, "mods")).filter((f) => f.endsWith(".jar"));
  r.check("every mod file is in mods/", mods.length > 10, `${mods.length} jars`);
  const top = readdirSync(dir);
  r.check("overrides were copied (config or options)", top.includes("config") || top.includes("options.txt"), JSON.stringify(top));
  r.check("the index itself is not copied", !existsSync(join(dir, "modrinth.index.json")));
  const recs = ok(await d.inv("list_mods", { instanceId: inst.id }), "list mods");
  r.check("mods are identified on Modrinth", recs.filter((m) => m.source === "modrinth").length >= mods.length - 2, `${recs.filter((m) => m.source === "modrinth").length}/${recs.length}`);
  const left = readdirSync(instancesDir);
  r.check("the cancelled instance's folder is gone", left.length === 1, JSON.stringify(left));

  // The New instance screen offers a file import.
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "New instance");
  await sleep(200);
  await d.key("Enter");
  r.check("New instance offers Import modpack", await d.waitFor(`[...document.querySelectorAll('#main button')].some(b=>/Import modpack/.test(b.textContent))`, 5000));
  ok(await d.inv("delete_instance", { id: inst.id }), "delete");
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
