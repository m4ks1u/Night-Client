// Data-folder relocation end to end (DEBUG build, Vite dev server running).
//
//   node e2e/relocate.mjs <path-to-debug-night-client.exe>
//
// Runs against a throwaway APPDATA/LOCALAPPDATA in the temp folder, never
// your real data. It moves a small seeded data folder out, checks the app
// restarted onto it, moves it back, and checks again. The app restarts
// itself after each move; at the end every night-client.exe started from
// this exe path is stopped.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { connect, launch, reporter, sleep } from "./driver.mjs";

const exe = resolve(process.argv[2] ?? "");
const port = 9357;
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-relocate-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const home = join(env.APPDATA, "NightClient");
const moved = join(tmp, "Other drive", "Night Client");
const files = ["versions/1.21.1/1.21.1.json", "libraries/org/x/x.jar", "instances/World/saves/level.dat"];

for (const f of files) {
  mkdirSync(dirname(join(home, f)), { recursive: true });
  writeFileSync(join(home, f), `seed:${f}`);
}

const pageIds = async () => {
  try {
    return (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).filter((p) => p.type === "page").map((p) => p.id);
  } catch {
    return [];
  }
};

/**
 * Ask for a move, then attach to the restarted app: wait until a DevTools
 * page with a new id appears (the old process is gone by then).
 */
async function moveAndReattach(old, target) {
  const before = await pageIds();
  void old.inv("relocate_data", { target }); // never resolves: the app restarts
  for (let i = 0; i < 60; i++) {
    await sleep(500);
    const now = await pageIds();
    if (now.length > 0 && !now.some((id) => before.includes(id))) break;
  }
  old.detach();
  await sleep(1500);
  return connect(port);
}

const allThere = (root) => files.every((f) => existsSync(join(root, f)) && readFileSync(join(root, f), "utf8") === `seed:${f}`);

let d = await launch(exe, { port, env });
let failed = false;
try {
  const info = (await d.inv("data_folder_info")).v;
  r.check("starts in the default folder", info?.isDefault === true && info.path === home, JSON.stringify(info));

  // Refusals: network share, inside the current folder, a non-empty folder.
  mkdirSync(join(tmp, "Full"));
  writeFileSync(join(tmp, "Full", "x.txt"), "x");
  for (const [name, target] of [
    ["network share", "\\\\server\\share\\nc"],
    ["inside current", join(home, "sub")],
    ["non-empty folder", join(tmp, "Full")],
  ]) {
    const res = await d.inv("relocate_data", { target });
    r.check(`refuses ${name}`, res.ok === false && res.e?.code === "invalid_path", JSON.stringify(res.e));
  }
  r.check("nothing moved by refusals", allThere(home));

  // The UI shows the folder and the confirmation dialog.
  await d.js(`document.querySelector('nav button[aria-label=Settings], a[href="#settings"]')?.click(); true`);
  await d.key(",", 2);
  r.check("settings shows the data folder", await d.waitFor(`document.body.innerText.includes(${JSON.stringify(home)})`, 8000));

  // Move out; the app restarts on success.
  d = await moveAndReattach(d, moved);
  r.check("files moved to the new folder", allThere(moved));
  r.check("old game-data folders removed", files.every((f) => !existsSync(join(home, f.split("/")[0]))));
  r.check("location saved next to the database", existsSync(join(home, "data-location.json")));
  r.check("database stayed in the default folder", existsSync(join(home, "night-client.db")) && !existsSync(join(moved, "night-client.db")));
  const after = (await d.inv("data_folder_info")).v;
  r.check("app restarted onto the new folder", after?.path === moved && after.isDefault === false, JSON.stringify(after));

  // Move back to the default.
  d = await moveAndReattach(d, null);
  r.check("files moved back", allThere(home));
  r.check("location file removed", !existsSync(join(home, "data-location.json")));
  const back = (await d.inv("data_folder_info")).v;
  r.check("app restarted onto the default folder", back?.isDefault === true, JSON.stringify(back));
  const errs = await d.errors();
  r.check("no uncaught UI errors", errs.length === 0, JSON.stringify(errs));
} catch (e) {
  failed = true;
  r.check("script finished", false, String(e.message));
} finally {
  d.close();
  // The restarted processes are not our children; stop those of this exe only.
  try {
    execFileSync("powershell", [
      "-NoProfile",
      "-Command",
      `Get-Process night-client -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq '${exe.replace(/'/g, "''")}' } | Stop-Process -Force`,
    ]);
  } catch {}
  await sleep(1000);
  if (failed) console.log(`kept for inspection: ${tmp}`);
  else rmSync(tmp, { recursive: true, force: true });
}
process.exit(r.print());
