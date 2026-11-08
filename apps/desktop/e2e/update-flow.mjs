// Updater end to end: install, refuse a wrongly signed update, apply a good
// one, uninstall. Needs two DEBUG installers signed with a THROWAWAY key and
// the Vite dev server running; see docs/dev-setup.md ("Update flow").
//
//   node e2e/update-flow.mjs <dir containing 0.0.1/ and 0.0.2/ installers>
//
// It installs into <dir>/installed (current user) and uninstalls at the end.
import { createServer } from "node:http";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { connect, reporter, sleep } from "./driver.mjs";

const K = process.argv[2];
if (!K) throw new Error("usage: node e2e/update-flow.mjs <installer dir>");
const installDir = join(K, "installed").replaceAll("/", "\\");
const port = 9361;
const r = reporter();
const setup = (v) => join(K, v, `Night Client_${v}_x64-setup.exe`);
const sig = (v) => readFileSync(setup(v) + ".sig", "utf8").trim();
const feed = (signature) =>
  JSON.stringify({
    version: "0.0.2",
    notes: "Test release notes.\n<b>Shown as text, not HTML.</b>",
    pub_date: new Date().toISOString(),
    platforms: { "windows-x86_64": { signature, url: "http://127.0.0.1:8737/setup-0.0.2.exe" } },
  });
let latest = feed(sig("0.0.1")); // wrong signature first
const hits = [];
const server = createServer((req, res) => {
  hits.push(req.url);
  if (req.url === "/latest.json") {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(latest);
  } else if (req.url === "/setup-0.0.2.exe") {
    res.writeHead(200, { "content-length": statSync(setup("0.0.2")).size });
    createReadStream(setup("0.0.2")).pipe(res);
  } else {
    res.writeHead(404).end();
  }
}).listen(8737, "127.0.0.1");

const uninstallKey = () => {
  try {
    return execFileSync("powershell", ["-NoProfile", "-Command",
      "Get-ChildItem HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall | Get-ItemProperty | ? { $_.DisplayName -like '*Night Client*' } | % { $_.DisplayVersion }"]).toString().trim();
  } catch { return ""; }
};
const pageIds = async () => { try { return (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).filter((p) => p.type === "page").map((p) => p.id); } catch { return []; } };

let d;
try {
  const inst = spawnSync(setup("0.0.1"), ["/S", `/D=${installDir}`]);
  r.check("0.0.1 installs silently", inst.status === 0 && uninstallKey() === "0.0.1", `status=${inst.status} reg=${uninstallKey()}`);
  const exe = ["night-client.exe", "Night Client.exe"].map((n) => join(installDir, n)).find(existsSync);
  r.check("installed executable found", !!exe, exe);

  spawn(exe, [], { env: { ...process.env, NIGHT_CLIENT_DEVTOOLS_PORT: String(port) }, stdio: "ignore", detached: true }).unref();
  for (let i = 0; i < 40 && (await pageIds()).length === 0; i++) await sleep(500);
  d = await connect(port);

  let info = (await d.inv("check_for_update")).v;
  r.check("update found", info?.configured && info.currentVersion === "0.0.1" && info.available?.version === "0.0.2", JSON.stringify(info));
  r.check("notes are passed as plain text", info?.available?.notes.includes("<b>Shown as text"));

  const bad = await d.inv("install_update");
  r.check("wrongly signed update is refused", bad.ok === false && /verif|signature/i.test(bad.e?.message ?? ""), JSON.stringify(bad.e));
  r.check("app keeps running after the refusal", (await d.js("1+1")) === 2);
  r.check("still 0.0.1 after the refusal", uninstallKey() === "0.0.1");

  latest = feed(sig("0.0.2"));
  info = (await d.inv("check_for_update")).v;
  const before = await pageIds();
  void d.inv("install_update"); // the installer closes the app
  const procs = () => {
    try {
      return execFileSync("powershell", ["-NoProfile", "-Command", `Get-CimInstance Win32_Process -Filter "Name='night-client.exe'" | ? { $_.ExecutablePath -like '${installDir}*' } | % { \"$($_.ProcessId) $($_.CommandLine)\" }`]).toString().trim();
    } catch { return ""; }
  };
  const oldPid = procs().split(" ")[0];
  let relaunched = false;
  let seen = "";
  for (let i = 0; i < 90; i++) {
    await sleep(1000);
    const now = await pageIds();
    if (now.length && !now.some((id) => before.includes(id))) { relaunched = true; break; }
    const p = procs();
    if (p && p.split(" ")[0] !== oldPid && uninstallKey() === "0.0.2") { seen = p; }
  }
  d.detach();
  r.check("registry shows 0.0.2 after update", uninstallKey() === "0.0.2", uninstallKey());
  r.check("installer relaunched the app (new process from the install folder)", relaunched || !!seen || !!procs(), `devtools=${relaunched} proc=${seen || procs()}`);
  if (relaunched) {
    await sleep(1500);
    d = await connect(port);
    const after = (await d.inv("check_for_update")).v;
    r.check("running version is 0.0.2 and up to date", after?.currentVersion === "0.0.2" && after.available === null, JSON.stringify(after));
    d.detach();
  }
  r.check("feed and installer were requested", hits.includes("/latest.json") && hits.includes("/setup-0.0.2.exe"), JSON.stringify([...new Set(hits)]));
} catch (e) {
  r.check("script finished", false, e.message);
} finally {
  try { execFileSync("powershell", ["-NoProfile", "-Command", `Get-Process | ? { $_.Path -like '${installDir}*' } | Stop-Process -Force`]); } catch {}
  await sleep(1000);
  const un = join(installDir, "uninstall.exe");
  if (existsSync(un)) spawnSync(un, ["/S"]);
  for (let i = 0; i < 20 && uninstallKey(); i++) await sleep(500);
  r.check("uninstalled cleanly", uninstallKey() === "" && !existsSync(join(installDir, "night-client.exe")), uninstallKey());
  server.close();
}
process.exit(r.print());
