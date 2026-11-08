// Turning packs on from the launcher, against the real Modrinth API (DEBUG
// build, Vite dev server running), on a throwaway APPDATA/LOCALAPPDATA.
// Needs network; downloads a few packs and Iris + Sodium.
//
//   node e2e/pack-activation.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// 1. Vanilla: two resource packs are turned on (options.txt, in order, other
//    lines kept), one off, and removing an active pack drops it from
//    options.txt. A core-shader pack lands in resourcepacks/.
// 2. Fabric: a shader pack can't be turned on without Iris; after Iris is
//    installed it can, through config/iris.properties, and off again.
// 3. UI: the Resource packs tab has working "In game" switches.
// Nothing is launched.

import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-activate-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};
const instDir = (i) => join(env.APPDATA, "NightClient", "instances", i.dirName);

async function create(d, name, loader, loaderVersion = null) {
  return ok(await d.inv("create_instance", { input: { name, gameVersion: "1.21.1", loader, loaderVersion, maxMemoryMb: null } }), `create ${name}`);
}

/** Install the n-th search hit of `kind` (and whatever its plan needs). */
async function installHit(d, instanceId, kind, n, query = "") {
  const s = ok(await d.inv("modrinth_search", { instanceId, query, category: null, sort: "downloads", offset: 0, kind }), `search ${kind}`);
  return installProject(d, instanceId, kind, s.hits[n].project_id);
}

async function installProject(d, instanceId, kind, projectId) {
  const plan = ok(await d.inv("mod_install_plan", { instanceId, projectId, kind }), `plan ${projectId}`);
  const items = plan.install.map((p) => ({ versionId: p.version.id, requiredBy: p.requiredBy ?? null }));
  return ok(await d.inv("mod_install", { instanceId, items, kind }), `install ${projectId}`);
}

let d;
try {
  d = await launch(exe, { port: 9369, env, outDir });
  const vanilla = await create(d, "Activate vanilla", "vanilla");
  const vdir = instDir(vanilla);

  // 1. Resource packs on vanilla.
  const [a] = await installHit(d, vanilla.id, "resourcepack", 0);
  const [b] = await installHit(d, vanilla.id, "resourcepack", 1);
  let act = ok(await d.inv("pack_activation", { instanceId: vanilla.id, kind: "resourcepack" }), "activation");
  r.check("nothing is on at first", act.available && act.active.length === 0, JSON.stringify(act));
  writeFileSync(join(vdir, "options.txt"), "version:3955\nfov:0.25\nresourcePacks:[\"vanilla\"]\nlang:en_us\n");
  ok(await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "resourcepack", fileName: a.fileName, active: true }), "on a");
  ok(await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "resourcepack", fileName: b.fileName, active: true }), "on b");
  let opts = readFileSync(join(vdir, "options.txt"), "utf8");
  const line = opts.split("\n").find((l) => l.startsWith("resourcePacks:"));
  r.check("options.txt lists both packs, last on top", line === `resourcePacks:${JSON.stringify(["vanilla", `file/${a.fileName}`, `file/${b.fileName}`])}`, line);
  r.check("other options are kept, in order", opts.startsWith("version:3955\nfov:0.25\n") && opts.includes("lang:en_us\n"));
  act = ok(await d.inv("pack_activation", { instanceId: vanilla.id, kind: "resourcepack" }), "activation 2");
  r.check("activation reads them back", JSON.stringify(act.active) === JSON.stringify([a.fileName, b.fileName]), JSON.stringify(act.active));
  ok(await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "resourcepack", fileName: a.fileName, active: false }), "off a");
  act = ok(await d.inv("pack_activation", { instanceId: vanilla.id, kind: "resourcepack" }), "activation 3");
  r.check("turning one off leaves the other", JSON.stringify(act.active) === JSON.stringify([b.fileName]));
  ok(await d.inv("remove_mod", { instanceId: vanilla.id, fileName: b.fileName, kind: "resourcepack" }), "remove b");
  opts = readFileSync(join(vdir, "options.txt"), "utf8");
  r.check("removing an active pack drops it from options.txt", !opts.includes(b.fileName) && opts.includes("resourcePacks:[\"vanilla\"]"), opts.split("\n").find((l) => l.startsWith("resourcePacks:")));
  const bad = await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "resourcepack", fileName: "../x.zip", active: true });
  r.check("an unsafe file name is refused", !bad.ok);
  const missing = await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "resourcepack", fileName: "nope.zip", active: true });
  r.check("a pack that isn't installed can't be turned on", !missing.ok && missing.e?.code === "not_found");
  const modOn = await d.inv("set_pack_active", { instanceId: vanilla.id, kind: "mod", fileName: "x.jar", active: true });
  r.check("mods are refused", !modOn.ok);

  // Core shaders on vanilla go to resourcepacks/.
  const vs = ok(await d.inv("modrinth_search", { instanceId: vanilla.id, query: "", category: null, sort: "downloads", offset: 0, kind: "shader" }), "vanilla shaders");
  if (vs.hits.length > 0) {
    const [cs] = await installProject(d, vanilla.id, "shader", vs.hits[0].project_id);
    r.check("a core-shader pack is stored as a resource pack", cs.kind === "resourcepack" && existsSync(join(vdir, "resourcepacks", cs.fileName)), `${cs.kind} ${cs.fileName}`);
    r.check("nothing goes to shaderpacks/ on vanilla", !existsSync(join(vdir, "shaderpacks")) || readdirSync(join(vdir, "shaderpacks")).length === 0);
    const again = ok(await d.inv("modrinth_search", { instanceId: vanilla.id, query: "", category: null, sort: "downloads", offset: 0, kind: "shader" }), "vanilla shaders again");
    r.check("the shader search marks it installed", again.hits.find((h) => h.project_id === vs.hits[0].project_id)?.installed === true);
  } else {
    r.check("vanilla core-shader search returned results", false, "no hits; skipped");
  }
  const vshAct = ok(await d.inv("pack_activation", { instanceId: vanilla.id, kind: "shader" }), "vanilla shader activation");
  r.check("vanilla Shaders explains there is no shader mod", !vshAct.available && /Iris/.test(vshAct.note ?? ""));

  // 2. Shaders on Fabric.
  const fv = ok(await d.inv("loader_versions", { loader: "fabric", minecraft: "1.21.1" }), "fabric versions");
  const fabric = await create(d, "Activate fabric", "fabric", (fv.find((v) => v.stable) ?? fv[0]).version);
  const fdir = instDir(fabric);
  const [sh] = await installHit(d, fabric.id, "shader", 0);
  act = ok(await d.inv("pack_activation", { instanceId: fabric.id, kind: "shader" }), "shader activation");
  r.check("without Iris shaders can't be turned on here", !act.available && /Install Iris/.test(act.note ?? ""), act.note);
  const noIris = await d.inv("set_pack_active", { instanceId: fabric.id, kind: "shader", fileName: sh.fileName, active: true });
  r.check("turning one on without Iris is refused", !noIris.ok && noIris.e?.code === "no_iris");
  const iris = await installProject(d, fabric.id, "mod", "YL57xq9U");
  r.check("Iris (and its dependency) installed", iris.some((m) => m.projectId === "YL57xq9U"), iris.map((m) => m.fileName).join(", "));
  act = ok(await d.inv("pack_activation", { instanceId: fabric.id, kind: "shader" }), "shader activation 2");
  r.check("with Iris, shaders can be turned on", act.available && act.note === null);
  ok(await d.inv("set_pack_active", { instanceId: fabric.id, kind: "shader", fileName: sh.fileName, active: true }), "shader on");
  const props = readFileSync(join(fdir, "config", "iris.properties"), "latin1");
  r.check("iris.properties names the pack, shaders on", props.includes("enableShaders=true") && /shaderPack=/.test(props), props.trim().replaceAll("\n", " | "));
  act = ok(await d.inv("pack_activation", { instanceId: fabric.id, kind: "shader" }), "shader activation 3");
  r.check("the shader reads back as on", JSON.stringify(act.active) === JSON.stringify([sh.fileName]));
  ok(await d.inv("set_pack_active", { instanceId: fabric.id, kind: "shader", fileName: sh.fileName, active: false }), "shader off");
  act = ok(await d.inv("pack_activation", { instanceId: fabric.id, kind: "shader" }), "shader activation 4");
  r.check("and off again", act.active.length === 0 && readFileSync(join(fdir, "config", "iris.properties"), "latin1").includes("enableShaders=false"));

  // 3. UI.
  await d.reload();
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "Activate vanilla");
  await sleep(200);
  await d.key("Enter");
  await d.waitFor(`!!document.querySelector('[role=tab]')`, 5000);
  await d.click(/^\s*Resource packs\s*$/i, "[role=tab]");
  r.check("pack rows have an In game switch", await d.waitFor(`document.querySelectorAll('[aria-label="Installed resource packs"] [role=switch][id^="active-"]').length >= 1`, 15000));
  const sw = `document.querySelector('[aria-label="Installed resource packs"] [role=switch][id^="active-"]')`;
  const before = await d.js(`${sw}.getAttribute('aria-checked')`);
  await d.js(`${sw}.click(); 1`);
  r.check("the switch changes the game's settings", await d.waitFor(`${sw}.getAttribute('aria-checked') !== ${JSON.stringify(before)}`, 10000));
  r.check("the tab explains the switch", /In game/.test(await d.js(`document.getElementById('main').innerText`)));
  await sleep(600);
  await d.shot("pack-activation");

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
