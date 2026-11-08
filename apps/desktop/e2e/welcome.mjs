// First-run welcome (DEBUG build, Vite dev server running), on a throwaway
// APPDATA/LOCALAPPDATA so the welcome shows as for a new user.
//
//   node e2e/welcome.mjs <path-to-debug-night-client.exe> [screenshot-dir]

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
const tmp = mkdtempSync(join(tmpdir(), "nc-welcome-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const dialogText = `document.querySelector("dialog[open]")?.innerText || ""`;

async function axe(d, name) {
  await d.js(`window.axe ? true : (eval(${JSON.stringify(AXE)}), true)`);
  const v = await d.js(`axe.run(document, { runOnly: { type: "tag", values: ["wcag2a","wcag2aa","wcag21a","wcag21aa"] }, resultTypes: ["violations"] }).then(r => r.violations.map(v => v.id + ":" + v.nodes.length))`);
  r.check(`axe: ${name}`, v.length === 0, JSON.stringify(v));
  await d.shot(`welcome-${name}`);
}

try {
  let d = await launch(exe, { port: 9363, env, outDir, welcome: "keep" });
  r.check("welcome shows on first start", await d.waitFor(`/Welcome to Night Client/.test(${dialogText})`, 8000));
  r.check("focus is inside the welcome", await d.js(`document.querySelector("dialog[open]")?.contains(document.activeElement)`));
  await axe(d, "step-1");

  await d.click(`/^Next$/`, "dialog[open] button");
  await sleep(300);
  r.check("step 2 shows the data folder", await d.js(`${dialogText}.includes(${JSON.stringify(join(env.APPDATA, "NightClient"))})`));
  await d.click(`/^Next$/`, "dialog[open] button");
  await sleep(300);
  r.check("step 3 explains connections", await d.js(`/no analytics or telemetry/i.test(${dialogText})`));
  await axe(d, "step-3");
  await d.click(`/Check for Night Client updates/`, "dialog[open] button[role=switch]");
  await sleep(200);
  r.check("update toggle switched off", await d.js(`document.querySelector("dialog[open] [role=switch]")?.getAttribute("aria-checked") === "false"`));
  await d.click(`/^Back$/`, "dialog[open] button");
  await sleep(200);
  r.check("Back works", await d.js(`/Your game data/.test(${dialogText})`));
  await d.click(`/^Next$/`, "dialog[open] button");
  await d.click(`/^Next$/`, "dialog[open] button");
  await sleep(300);
  await axe(d, "step-4");
  await d.click(`/Create an instance/`, "dialog[open] button");
  await sleep(800);
  r.check("finishing closes the welcome", !(await d.js(`!!document.querySelector("dialog[open]")`)));
  r.check("'Create an instance' opens New instance", await d.js(`/New instance/.test(document.querySelector("main h1")?.textContent || "")`));
  const s = (await d.inv("get_settings")).v;
  r.check("choices saved (onboarded, updates off)", s?.onboarded === true && s.checkUpdates === false, JSON.stringify(s));
  d.close();
  await sleep(1000);

  d = await launch(exe, { port: 9363, env, outDir, welcome: "keep" });
  await sleep(2500);
  r.check("welcome not shown again", !(await d.js(`!!document.querySelector("dialog[open]")`)));
  await d.key(",", 2);
  await sleep(600);
  await d.click(`/Show welcome again/`);
  r.check("Settings can reopen it", await d.waitFor(`/Welcome to Night Client/.test(${dialogText})`, 3000));
  await d.key("Escape");
  await sleep(500);
  r.check("Esc closes it", !(await d.js(`!!document.querySelector("dialog[open]")`)));
  r.check("settings unchanged by reopening", (await d.inv("get_settings")).v?.checkUpdates === false);
  const errs = await d.errors();
  r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
  d.close();
} catch (e) {
  r.check("script finished", false, e.message);
} finally {
  await sleep(1000);
  rmSync(tmp, { recursive: true, force: true });
}
process.exit(r.print());
