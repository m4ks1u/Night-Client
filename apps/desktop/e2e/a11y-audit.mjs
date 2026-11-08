// Accessibility audit: runs axe-core (WCAG 2.1 A/AA rules) on every screen
// and the main dialogs of a DEBUG build.
//
//   node e2e/a11y-audit.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// Creates a temporary Fabric instance (vanilla files must already be
// cached for 1.21.1) and removes it afterwards.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const [exe, outDir = "."] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
const AXE = readFileSync(join(here, "..", "node_modules", "axe-core", "axe.min.js"), "utf8");
const d = await launch(exe, { port: 9351, outDir });
const r = reporter();

async function audit(name) {
  await d.js(`window.axe ? true : (eval(${JSON.stringify(AXE)}), true)`);
  const res = await d.js(`axe.run(document, { runOnly: { type: "tag", values: ["wcag2a","wcag2aa","wcag21a","wcag21aa"] }, resultTypes: ["violations"] })
    .then(r => r.violations.map(v => ({ id: v.id, impact: v.impact, n: v.nodes.length, sample: v.nodes.slice(0,2).map(n => n.target.join(" ") + " :: " + (n.failureSummary||"").split("\\n").slice(1,2).join(" ")) })))`);
  r.check(`axe: ${name}`, res.length === 0, res.length ? "\n    " + res.map((v) => `${v.id} (${v.impact}, ${v.n}): ${v.sample.join(" | ")}`).join("\n    ") : "");
  await d.shot(`a11y-${name.replace(/\W+/g, "-")}`);
}

const inst = (await d.inv("create_instance", { input: { name: "A11y Audit", gameVersion: "1.21.1", loader: "fabric", loaderVersion: "0.20.1", maxMemoryMb: null } })).v;
await d.reload();

for (const [label, screen] of [["Home", "home"], ["Instances", "instances"], ["Discover", "discover"], ["Downloads", "downloads"], ["Settings", "settings"], ["Diagnostics", "diagnostics"]]) {
  await d.click(`/^${label}/`, "nav button");
  await sleep(1500);
  await audit(screen);
}
await d.click(`/Not signed in|Sign in/`, "nav button");
await sleep(800);
await audit("accounts");

await d.click(`/^Instances/`, "nav button");
await sleep(500);
await d.click(`/^New instance$/`);
await sleep(1500);
await audit("new-instance");

await d.click(`/^Instances/`, "nav button");
await sleep(500);
await d.click(`/A11y Audit/`, "main li button, main a, main [role=link], main article button");
await sleep(1000);
await audit("instance-overview");
await d.click(`/^Mods$/`, "[role=tab]");
await sleep(1200);
await audit("instance-mods");
await d.click(`/^Game log$/`, "[role=tab]");
await sleep(500);
await audit("instance-log");
await d.click(`/^Overview$/`, "[role=tab]");
await sleep(500);
await d.click(`/^Delete instance$/`);
await sleep(400);
await audit("dialog-delete");
await d.click(`/^Cancel$/`, "dialog[open] button");

const del = await d.inv("delete_instance", { id: inst.id });
r.check("test instance deleted", del.ok, del.ok ? "" : JSON.stringify(del.e));
const errs = await d.errors();
r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
d.close();
process.exit(r.print());
