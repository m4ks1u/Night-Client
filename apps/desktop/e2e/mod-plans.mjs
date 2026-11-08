// Install plans against the real Modrinth API (DEBUG build, Vite dev server
// running), on a throwaway APPDATA/LOCALAPPDATA. Needs network access.
//
//   node e2e/mod-plans.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// 1. Fabric 1.21.1: Sodium Extra declares optional dependencies. They are
//    offered as options, not installed by default, and a chosen one is
//    installed with its own requirements, each project once.
// 2. Quilt 1.21.1: a Fabric-only mod (Sodium) is offered and plans cleanly.
// 3. Forge 1.20.1: a Forge mod (JEI) plans; Forge for 1.16.5 is not offered.
// Nothing is launched; only mod jars (a few MB) are downloaded.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-plans-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};

async function instance(d, loader, mc = "1.21.1") {
  const versions = ok(await d.inv("loader_versions", { loader, minecraft: mc }), `${loader} versions`);
  const lv = (versions.find((v) => v.stable) ?? versions[0])?.version;
  r.check(`${loader} has loader versions for ${mc}`, !!lv, JSON.stringify(versions.slice(0, 3)));
  return ok(
    await d.inv("create_instance", { input: { name: `Plans ${loader}`, gameVersion: mc, loader, loaderVersion: lv, maxMemoryMb: null } }),
    `create ${loader}`,
  );
}

let d;
try {
  d = await launch(exe, { port: 9364, env, outDir });
  const fab = await instance(d, "fabric");
  const plan = ok(await d.inv("mod_install_plan", { instanceId: fab.id, projectId: "sodium-extra" }), "plan sodium-extra");
  const main = plan.install.map((m) => m.version.project_id);
  r.check("main plan: Sodium Extra plus required Sodium", main.length >= 2 && main.includes("AANobbMI"), JSON.stringify(main));
  r.check("optional dependencies listed", plan.optional.length > 0, JSON.stringify(plan.optional));
  r.check("optional ones are not in the main plan", !plan.optional.some((p) => main.includes(p)));
  r.check("installable options resolved with titles", plan.options.length > 0 && plan.options.every((o) => plan.titles[o.install[0].version.project_id]), JSON.stringify(plan.options.map((o) => o.install.map((m) => m.version.project_id))));
  r.check("options don't repeat planned mods", plan.options.every((o) => o.install.slice(1).every((m) => !main.includes(m.version.project_id))));

  // The dialog: options are listed, off by default; ticking one raises the count.
  await d.js("location.reload()");
  await sleep(2500);
  await d.click(/^\s*Discover\s*$/i, "a,button");
  await d.waitFor(`!!document.querySelector('input[type=search]')`, 8000);
  await d.setValue("input[type=search]", "sodium extra");
  const row = `document.querySelector('button[aria-label="Open Sodium Extra on Modrinth"]')?.closest('li')`;
  r.check("Discover lists Sodium Extra", await d.waitFor(`!!${row}`, 15000));
  await d.js(`[...${row}.querySelectorAll('button')].find(b=>/^Install$/i.test(b.textContent.trim()))?.click()`);
  const dlg = `document.querySelector('dialog[open]')`;
  r.check("dialog offers optional extras", await d.waitFor(`/Optional extras/i.test(${dlg}?.innerText||'')`, 20000));
  const boxes = `[...${dlg}.querySelectorAll('fieldset input[type=checkbox]')]`;
  r.check("extras are unticked by default", await d.js(`${boxes}.length>0 && ${boxes}.every(b=>!b.checked)`));
  const label = `[...${dlg}.querySelectorAll('button')].map(b=>b.textContent.trim()).find(t=>/^Install/.test(t))`;
  const before = await d.js(label);
  await d.js(`${boxes}[0].click()`);
  await sleep(300);
  const after = await d.js(label);
  r.check("ticking an extra adds it to the install", before === "Install 2 mods" && after === "Install 3 mods", `${before} → ${after}`);
  await d.shot("mod-plans-dialog");
  await d.js(`[...${dlg}.querySelectorAll('button')].find(b=>/^Cancel$/.test(b.textContent.trim())).click()`);

  // Install the main plan plus the first option, as the dialog would.
  const opt = plan.options[0];
  const items = [...plan.install, ...(opt?.install ?? [])].map((m) => ({ versionId: m.version.id, requiredBy: m.requiredBy }));
  const done = ok(await d.inv("mod_install", { instanceId: fab.id, items }), "install");
  const mods = ok(await d.inv("list_mods", { instanceId: fab.id }), "list mods");
  const want = new Set([...main, ...(opt?.install.map((m) => m.version.project_id) ?? [])]);
  r.check("every chosen mod installed once", done.length === want.size && mods.filter((m) => m.projectId && want.has(m.projectId)).length === want.size, `${done.length} installed, ${mods.length} listed, want ${want.size}`);
  const again = ok(await d.inv("mod_install_plan", { instanceId: fab.id, projectId: "sodium-extra" }), "re-plan");
  r.check("installed option is no longer offered", !again.optional.includes(opt?.install[0].version.project_id));

  const quilt = await instance(d, "quilt");
  const qp = ok(await d.inv("mod_install_plan", { instanceId: quilt.id, projectId: "sodium" }), "plan sodium on quilt");
  r.check("Quilt instance plans a Fabric mod", qp.install[0]?.version.project_id === "AANobbMI" && qp.install[0].version.files.length > 0);
  const search = ok(await d.inv("modrinth_search", { instanceId: quilt.id, query: "sodium", category: null, sort: "relevance", offset: 0 }), "search on quilt");
  r.check("Quilt search lists Fabric mods", search.hits.some((h) => h.slug === "sodium"), JSON.stringify(search.hits.slice(0, 5).map((h) => h.slug)));

  const forge = await instance(d, "forge", "1.20.1");
  const fp = ok(await d.inv("mod_install_plan", { instanceId: forge.id, projectId: "jei" }), "plan jei on forge");
  r.check("Forge 1.20.1 instance plans a Forge mod (JEI)", fp.install[0]?.version.project_id === "u6dRKJwZ" && fp.install[0].version.loaders?.includes("forge"), JSON.stringify(fp.install[0]?.version.loaders));
  const f165 = ok(await d.inv("loader_versions", { loader: "forge", minecraft: "1.16.5" }), "forge 1.16.5");
  r.check("Forge 1.16.5 is offered (spec 0 installer)", f165.length > 0 && f165[0].version.startsWith("36."), JSON.stringify(f165.slice(0, 2)));
  const f1122 = ok(await d.inv("loader_versions", { loader: "forge", minecraft: "1.12.2" }), "forge 1.12.2");
  const build = (v) => Number(v.split(".")[3]);
  r.check("Forge 1.12.2 lists only builds 2851 and newer", f1122.length > 0 && f1122.every((v) => v.version.startsWith("14.23.5.") && build(v.version) >= 2851), JSON.stringify(f1122.slice(-2)));
  const f1112 = ok(await d.inv("loader_versions", { loader: "forge", minecraft: "1.11.2" }), "forge 1.11.2");
  r.check("Forge 1.11.2 is not offered", f1112.length === 0);

  for (const i of [fab, quilt, forge]) ok(await d.inv("delete_instance", { id: i.id }), "delete");
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
