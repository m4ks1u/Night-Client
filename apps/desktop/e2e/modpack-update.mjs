// Modpack updates against the real Modrinth API and CDN (DEBUG build, Vite
// dev server running), on a throwaway APPDATA/LOCALAPPDATA. Needs network;
// downloads two versions of Fabulously Optimized (~90 MB in total).
//
//   node e2e/modpack-update.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// 1. An older version of the pack is imported; the instance remembers it.
// 2. The user changes things: edits a config file, deletes a mod, disables
//    a mod, adds a file of their own.
// 3. A cancelled update changes nothing. The real update keeps the user's
//    changes, moves the instance to the new version, and a second check
//    reports it up to date.
// 4. UI: the Overview tab shows the Modpack card and its update check.
// Nothing is launched.

import { existsSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exeArg, outDir = "."] = process.argv.slice(2);
const exe = resolve(exeArg ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-mpupdate-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const ok = (res, what) => {
  if (!res?.ok) throw new Error(`${what}: ${JSON.stringify(res?.e)}`);
  return res.v;
};
const PACK = "fabulously-optimized";
const LOADERS = ["fabric", "quilt", "neoforge", "forge"];

/** The version Night Client would pick, and the oldest one for the same Minecraft version (the biggest difference). */
async function pickVersions() {
  const res = await fetch(`https://api.modrinth.com/v2/project/${PACK}/version`, { headers: { "User-Agent": "NightClient-e2e/0.1 (+https://github.com)" } });
  const all = (await res.json()).filter(
    (v) => v.version_type === "release" && v.loaders.some((l) => LOADERS.includes(l)) && (v.files.find((f) => f.primary) ?? v.files[0])?.filename.endsWith(".mrpack"),
  );
  const latest = all[0];
  const older = all.slice(1).filter((v) => v.game_versions.some((g) => latest.game_versions.includes(g))).at(-1) ?? all[1];
  return { latest, older };
}

let d;
try {
  const { latest, older } = await pickVersions();
  r.check("found two pack versions", !!latest && !!older, `${older?.version_number} → ${latest?.version_number}`);
  d = await launch(exe, { port: 9368, env, outDir });

  // 1. Import the older version.
  const p = ok(await d.inv("modpack_preview_modrinth", { projectId: PACK, versionId: older.id }), "preview old");
  r.check("preview uses the requested version", p.versionId === older.version_number, `${p.versionId}`);
  const other = await d.inv("modpack_preview_modrinth", { projectId: "sodium", versionId: older.id });
  r.check("a version of another project is refused", !other.ok);
  const inst = ok(await d.inv("modpack_import", { token: p.token, name: "FO update" }), "import");
  const info = ok(await d.inv("modpack_info", { instanceId: inst.id }), "info");
  r.check("instance remembers the pack", info?.projectId && info.versionId === older.id && info.versionNumber === older.version_number, JSON.stringify(info));
  const dir = join(env.APPDATA, "NightClient", "instances", inst.dirName);

  // 2. User changes.
  const configs = readdirSync(join(dir, "config"), { recursive: true }).map(String).filter((f) => /\.(json5?|toml|properties|txt)$/.test(f));
  const edited = join(dir, "config", configs[0]);
  writeFileSync(edited, "# edited by the e2e test\n");
  const jars = readdirSync(join(dir, "mods")).filter((f) => f.endsWith(".jar")).sort();
  const [deleted, disabled] = [jars[0], jars[1]];
  unlinkSync(join(dir, "mods", deleted));
  renameSync(join(dir, "mods", disabled), join(dir, "mods", `${disabled}.disabled`));
  writeFileSync(join(dir, "mods", "zz-own-notes.txt"), "mine");
  r.check("set up user changes", existsSync(edited), `${configs[0]}, -${deleted}, off ${disabled}`);

  // 3. Update.
  const avail = ok(await d.inv("modpack_check_update", { instanceId: inst.id }), "check");
  r.check("the newer version is offered", avail?.versionId === latest.id, JSON.stringify(avail));
  await d.js(`window.__upd = window.__TAURI_INTERNALS__.invoke("modpack_update", { instanceId: ${JSON.stringify(inst.id)}, versionId: ${JSON.stringify(latest.id)} }).then(v=>({ok:true,v}),e=>({ok:false,e})); 1`);
  await sleep(150);
  ok(await d.inv("modpack_cancel", { token: inst.id }), "cancel");
  const cancelled = await d.js("window.__upd");
  r.check("a cancelled update reports cancelled", !cancelled.ok && cancelled.e?.code === "cancelled", JSON.stringify(cancelled.e ?? "finished before cancel"));
  const still = ok(await d.inv("modpack_info", { instanceId: inst.id }), "info after cancel");
  r.check("a cancelled update changes nothing", still.versionId === older.id && readFileSync(edited, "utf8").startsWith("# edited") && !existsSync(join(dir, ".nc-update")));

  const up = ok(await d.inv("modpack_update", { instanceId: inst.id, versionId: latest.id }), "update");
  const rep = up.report;
  r.check("update reports its changes", rep.added + rep.replaced + rep.removed + rep.unchanged > 10, JSON.stringify({ ...rep, kept: rep.kept.length }));
  r.check("the edited config is kept", readFileSync(edited, "utf8").startsWith("# edited") && rep.kept.some((k) => k.endsWith(configs[0].replaceAll("\\", "/"))), configs[0]);
  r.check("the deleted mod stays deleted", !existsSync(join(dir, "mods", deleted)), deleted);
  r.check("the disabled mod is not re-enabled", !existsSync(join(dir, "mods", disabled)), disabled);
  r.check("the user's own file is untouched", readFileSync(join(dir, "mods", "zz-own-notes.txt"), "utf8") === "mine");
  r.check("no staging folder is left", !existsSync(join(dir, ".nc-update")));
  const after = ok(await d.inv("modpack_info", { instanceId: inst.id }), "info after");
  r.check("instance is on the new version", after.versionId === latest.id && after.versionNumber === latest.version_number, after.versionNumber);
  r.check("instance follows the pack's Minecraft and loader", latest.game_versions.includes(up.instance.gameVersion) && up.instance.loader === "fabric", `${up.instance.gameVersion} ${up.instance.loaderVersion}`);
  const none = ok(await d.inv("modpack_check_update", { instanceId: inst.id }), "check again");
  r.check("then it is up to date", none === null);
  const mods = ok(await d.inv("list_mods", { instanceId: inst.id }), "mods");
  r.check("Mods tab data follows the new files", mods.length > 10 && mods.every((m) => m.fileName !== deleted), `${mods.length} mods`);

  // 4. UI.
  await d.reload();
  await d.key("k", 2);
  await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000);
  await d.setValue("dialog[open] input[role=combobox]", "FO update");
  await sleep(200);
  await d.key("Enter");
  r.check("Overview shows the Modpack card", await d.waitFor(`[...document.querySelectorAll('#main h2')].some(h=>h.textContent==='Modpack') && /Version /.test(document.getElementById('main').innerText)`, 8000));
  await d.click(/^\s*Check for updates\s*$/i, "#main button");
  r.check("check reports up to date", await d.waitFor(`/You have the newest version/.test(document.getElementById('main').innerText)`, 20000));
  await sleep(500);
  await d.shot("modpack-card");

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
