// Resource packs and shader packs against the real Modrinth API (DEBUG build,
// Vite dev server running), on a throwaway APPDATA/LOCALAPPDATA. Needs network.
//
//   node e2e/packs.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// 1. Vanilla 1.21.1: resource packs are searched, planned (no dependency
//    resolution), installed into resourcepacks/, disabled, re-enabled and
//    removed. Mods stay refused on vanilla.
// 2. Fabric 1.21.1: shader packs for Iris/OptiFine are searched and one is
//    installed into shaderpacks/; it does not show up as a mod.
// 3. UI: the instance page has Resource packs and Shaders tabs listing the
//    installed files, and Discover switches between content types.
// Nothing is launched; only a couple of pack files are downloaded.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-packs-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};

async function create(d, name, loader, loaderVersion = null) {
  return ok(await d.inv("create_instance", { input: { name, gameVersion: "1.21.1", loader, loaderVersion, maxMemoryMb: null } }), `create ${name}`);
}

/** Search by downloads, plan the top hit, install it; returns [hit, records]. */
async function installTop(d, instanceId, kind) {
  const s = ok(await d.inv("modrinth_search", { instanceId, query: "", category: null, sort: "downloads", offset: 0, kind }), `search ${kind}`);
  r.check(`${kind} search returns results`, s.hits.length > 0 && s.total > 0, `${s.total} total`);
  const hit = s.hits[0];
  const plan = ok(await d.inv("mod_install_plan", { instanceId, projectId: hit.project_id, kind }), `plan ${hit.slug}`);
  r.check(`${kind} plan is the pack alone`, plan.install.length === 1 && plan.options.length === 0 && plan.missing.length === 0, JSON.stringify(plan.install.map((m) => m.version.project_id)));
  const file = plan.install[0].version.files.find((f) => f.primary) ?? plan.install[0].version.files[0];
  r.check(`${kind} file is a .zip`, /\.zip$/i.test(file.filename), file.filename);
  const recs = ok(await d.inv("mod_install", { instanceId, items: [{ versionId: plan.install[0].version.id, requiredBy: null }], kind }), `install ${hit.slug}`);
  return [hit, recs];
}

let d;
try {
  d = await launch(exe, { port: 9366, env, outDir });
  const vanilla = await create(d, "Packs vanilla", "vanilla");
  const fv = ok(await d.inv("loader_versions", { loader: "fabric", minecraft: "1.21.1" }), "fabric versions");
  const fabric = await create(d, "Packs fabric", "fabric", (fv.find((v) => v.stable) ?? fv[0]).version);

  // 1. Resource packs on vanilla.
  const [rp, rpRecs] = await installTop(d, vanilla.id, "resourcepack");
  r.check("resource pack recorded as a resource pack", rpRecs.length === 1 && rpRecs[0].kind === "resourcepack" && rpRecs[0].projectId === rp.project_id, JSON.stringify(rpRecs.map((x) => [x.kind, x.fileName])));
  const file = rpRecs[0].fileName;
  let list = ok(await d.inv("list_mods", { instanceId: vanilla.id, kind: "resourcepack" }), "list packs");
  r.check("listed from resourcepacks/ and identified on Modrinth", list.length === 1 && list[0].fileName === file && list[0].source === "modrinth");
  const again = ok(await d.inv("modrinth_search", { instanceId: vanilla.id, query: "", category: null, sort: "downloads", offset: 0, kind: "resourcepack" }), "search again");
  r.check("search marks it installed", again.hits.find((h) => h.project_id === rp.project_id)?.installed === true);
  ok(await d.inv("set_mod_enabled", { instanceId: vanilla.id, fileName: file, enabled: false, kind: "resourcepack" }), "disable");
  list = ok(await d.inv("list_mods", { instanceId: vanilla.id, kind: "resourcepack" }), "list after disable");
  r.check("disabled pack stays listed, disabled", list.length === 1 && list[0].enabled === false);
  ok(await d.inv("set_mod_enabled", { instanceId: vanilla.id, fileName: file, enabled: true, kind: "resourcepack" }), "enable");
  const cats = ok(await d.inv("modrinth_categories", { kind: "resourcepack" }), "categories");
  r.check("resource pack categories load", cats.length > 0 && cats.every((c) => c.project_type === "resourcepack"), JSON.stringify(cats.slice(0, 3).map((c) => c.name)));
  const noMods = await d.inv("modrinth_search", { instanceId: vanilla.id, query: "", category: null, sort: "downloads", offset: 0 });
  r.check("mods are still refused on vanilla", !noMods.ok && noMods.e?.code === "no_loader", JSON.stringify(noMods.e));
  const vShaders = ok(await d.inv("modrinth_search", { instanceId: vanilla.id, query: "", category: null, sort: "downloads", offset: 0, kind: "shader" }), "vanilla shaders");
  r.check("vanilla shader search works (core shaders only)", Array.isArray(vShaders.hits), `${vShaders.total} total`);
  const badName = await d.inv("remove_mod", { instanceId: vanilla.id, fileName: "x.jar", kind: "resourcepack" });
  r.check("a .jar name is refused for resource packs", !badName.ok);

  // 2. Shader packs on Fabric.
  const [sh, shRecs] = await installTop(d, fabric.id, "shader");
  r.check("shader pack recorded as a shader", shRecs[0]?.kind === "shader" && shRecs[0].projectId === sh.project_id, `${sh.title}`);
  const fabMods = ok(await d.inv("list_mods", { instanceId: fabric.id }), "fabric mods");
  r.check("shader pack is not listed as a mod", fabMods.length === 0, JSON.stringify(fabMods.map((m) => m.fileName)));

  // 3. UI.
  await d.reload();
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "Packs vanilla");
  await sleep(200);
  await d.key("Enter");
  r.check("instance page opens", await d.waitFor(`!!document.querySelector('[role=tab]')`, 5000));
  const tabs = await d.js(`[...document.querySelectorAll('[role=tab]')].map(t=>t.textContent.trim())`);
  r.check("vanilla instance has Resource packs and Shaders tabs, no Mods tab", JSON.stringify(tabs) === JSON.stringify(["Overview", "Resource packs", "Shaders", "Game log"]), JSON.stringify(tabs));
  await d.click(/^\s*Resource packs\s*$/i, "[role=tab]");
  r.check("Resource packs tab lists the pack", await d.waitFor(`document.querySelectorAll('[aria-label="Installed resource packs"] li').length === 1`, 15000));
  r.check("tab explains how to turn packs on", await d.waitFor(`/Options → Resource Packs/.test(document.getElementById('main')?.innerText||'')`, 3000));
  r.check("no Add file button for packs", !(await d.js(`[...document.querySelectorAll('button')].some(b=>/^Add file$/.test(b.textContent.trim()))`)));
  await sleep(800);
  await d.shot("packs-tab");
  await d.click(/^\s*Browse Modrinth\s*$/i, "button");
  r.check("Browse opens Discover on resource packs", await d.waitFor(`document.querySelector('[aria-label="Content type"] [aria-pressed=true]')?.textContent.trim() === 'Resource packs'`, 5000));
  r.check("Discover shows resource pack results", await d.waitFor(`/Showing resource packs for/.test(document.getElementById('main')?.innerText||'') && document.querySelectorAll('#main ul li').length > 0`, 20000));
  await d.click(/^\s*Shaders\s*$/i, "[aria-label='Content type'] button");
  r.check("switching to Shaders updates the search", await d.waitFor(`/Showing shader packs for/.test(document.getElementById('main')?.innerText||'')`, 5000));
  r.check("Shaders is the pressed content type", (await d.js(`document.querySelector('[aria-label="Content type"] [aria-pressed=true]')?.textContent.trim()`)) === "Shaders");
  await d.waitFor(`document.querySelectorAll('#main ul li').length > 0`, 20000);
  await sleep(800);
  await d.shot("packs-discover");

  for (const i of [vanilla, fabric]) ok(await d.inv("delete_instance", { id: i.id }), "delete");
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
