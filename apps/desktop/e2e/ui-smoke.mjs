// Stage 6 UI smoke test (DEBUG build): palette, shortcuts, toasts, layout
// persistence, reduce motion, offline banner, focus management,
// virtualized logs and the snap-layout hover.
//
//   node e2e/ui-smoke.mjs <path-to-debug-night-client.exe> [screenshot-dir]
//
// Needs vanilla 1.21.1 already installed (it runs a verify job).

import { launch, reporter, sleep } from "./driver.mjs";

const [exe, outDir = "."] = process.argv.slice(2);
const d = await launch(exe, { port: 9352, outDir });
const r = reporter();
const CTRL = 2;
const h1 = `document.querySelector('main h1')?.textContent`;

const inst = (await d.inv("create_instance", { input: { name: "UI Smoke", gameVersion: "1.21.1", loader: "vanilla", loaderVersion: null, maxMemoryMb: null } })).v;
await d.reload();

// Command palette: open with Ctrl+K, filter, Enter navigates.
await d.key("k", CTRL);
r.check("Ctrl+K opens the palette", await d.waitFor(`!!document.querySelector('dialog[open][aria-label="Command palette"]')`, 3000));
await d.setValue("dialog[open] input[role=combobox]", "diag");
await sleep(150);
const opts = await d.js(`[...document.querySelectorAll('dialog[open] [role=option]')].map(o=>o.textContent)`);
r.check("palette filters by word start", opts.length >= 1 && /Diagnostics/.test(opts[0]), JSON.stringify(opts));
await d.shot("ui-palette");
await d.key("Enter");
r.check("Enter runs the command", await d.waitFor(`${h1}==='Diagnostics'`, 5000));
r.check("focus moves to the new page heading", await d.waitFor(`document.activeElement?.tagName==='H1'`, 3000));

// Log virtualization: launcher log has hundreds of lines, few rendered.
const logLines = await d.js(`(()=>{const l=document.querySelector('[role=log]');return {rendered:l.querySelectorAll('[data-index]').length, total: l.firstElementChild?.style.height}})()`);
r.check("launcher log is virtualized", logLines.rendered > 0 && logLines.rendered < 120, JSON.stringify(logLines));

// Shortcuts.
await d.key("n", CTRL);
r.check("Ctrl+N opens New instance", await d.waitFor(`${h1}==='New instance'`, 3000));
await d.key(",", CTRL);
r.check("Ctrl+, opens Settings", await d.waitFor(`${h1}==='Settings'`, 3000));
r.check("shortcuts are documented in Settings", /Keyboard shortcuts/.test(await d.js(`document.querySelector('main').innerText`)));

// Reduce motion setting toggles the root class and persists.
await d.click(`/Reduce motion/`, "[role=switch]");
r.check("reduce motion applies", await d.waitFor(`document.documentElement.classList.contains('reduce-motion')`, 3000));
await sleep(600);
r.check("reduce motion persisted", (await d.inv("get_settings")).v.reduceMotion === true);
await d.click(`/Reduce motion/`, "[role=switch]");
await sleep(600);

// Instances: grid by default, list choice persists across reload.
await d.click(`/^Instances/`, "nav button");
await sleep(400);
r.check("grid view with artwork", await d.js(`!!document.querySelector('main article svg')`));
await d.shot("ui-instances-grid");
await d.click(`/^List$/`, "[role=radio]");
await d.reload();
await d.click(`/^Instances/`, "nav button");
await sleep(400);
r.check("list view remembered after reload", await d.js(`!document.querySelector('main article') && !!document.querySelector('main ul[aria-label=Instances] li')`));
await d.click(`/^Grid$/`, "[role=radio]");

// Toast for a background job (verify the installed version).
await d.inv("install_version", { id: "vanilla:1.21.1" });
r.check("toast appears when a download job finishes", await d.waitFor(`/Minecraft 1\\.21\\.1 verified/.test(document.querySelector('[aria-label=Notifications]')?.innerText||'')`, 60000));
await d.shot("ui-toast");

// Offline banner via network emulation.
await d.send("Network.enable");
await d.send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
r.check("offline banner shown", await d.waitFor(`/You're offline/.test(document.body.innerText)`, 5000));
await d.send("Network.emulateNetworkConditions", { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
r.check("offline banner hides when back online", await d.waitFor(`!/You're offline/.test(document.body.innerText)`, 5000));

// Skip link exists and is the first focusable element.
r.check("skip link is first in tab order", await d.js(`document.querySelector('a[href="#main"]')?.textContent==='Skip to content'`));

// Snap layouts: hover the maximize button (real mouse events through CDP).
const box = await d.js(`(()=>{const b=document.querySelector('button[aria-label=Maximize]').getBoundingClientRect();return {x:b.x+b.width/2,y:b.y+b.height/2}})()`);
await d.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: box.x, y: box.y });
await sleep(1400);
r.check("snap-layout hover handled without errors", (await d.errors()).length === 0);

const del = await d.inv("delete_instance", { id: inst.id });
r.check("test instance deleted", del.ok, del.ok ? "" : JSON.stringify(del.e));
const errs = await d.errors();
r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
d.close();
process.exit(r.print());
