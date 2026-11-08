// Crash reporting end to end (DEBUG build, Vite dev server running), on a
// throwaway APPDATA/LOCALAPPDATA.
//
//   node e2e/crash-report.mjs <path-to-debug-night-client.exe>
//
// Run 1 panics on a side thread (NIGHT_CLIENT_TEST_PANIC, debug builds only)
// with a fake token in the message. The crash report must exist and be
// redacted. Run 2 must show the "closed unexpectedly" notice once; run 3 must not.

import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch, reporter, sleep } from "./driver.mjs";

const exe = resolve(process.argv[2] ?? "");
const r = reporter();
const tmp = mkdtempSync(join(tmpdir(), "nc-crash-"));
const env = { APPDATA: join(tmp, "Roaming"), LOCALAPPDATA: join(tmp, "Local") };
const logs = join(env.LOCALAPPDATA, "NightClient", "logs");
const notice = `/closed unexpectedly/.test(document.querySelector('[aria-label=Notifications]')?.innerText||'')`;

try {
  let d = await launch(exe, { port: 9362, env: { ...env, NIGHT_CLIENT_TEST_PANIC: "1" } });
  await sleep(5000); // the test panic fires 4 s after startup
  r.check("app survives a panic on a side thread", (await d.js("1+1")) === 2);
  d.close();
  await sleep(1000);

  const crashes = readdirSync(logs).filter((n) => n.startsWith("crash-"));
  r.check("crash report written", crashes.length === 1, JSON.stringify(crashes));
  const text = crashes[0] ? readFileSync(join(logs, crashes[0]), "utf8") : "";
  r.check("crash report names the panic and version", /test panic/.test(text) && /Night Client \d+\.\d+\.\d+ crashed/.test(text));
  r.check("token removed from the crash report", !text.includes("c2lnbmF0dXJlLXRlc3Q") && !text.includes("eyJzdWIiOiJ0ZXN0In0"), text.slice(0, 300));
  const log = readdirSync(logs).filter((n) => n.endsWith(".log")).map((n) => readFileSync(join(logs, n), "utf8")).join("\n");
  r.check("panic logged without the token", /Night Client crashed/.test(log) && !log.includes("c2lnbmF0dXJlLXRlc3Q"));

  d = await launch(exe, { port: 9362, env });
  r.check("next start shows the crash notice", await d.waitFor(notice, 8000));
  d.close();
  await sleep(1000);

  d = await launch(exe, { port: 9362, env });
  await sleep(2500);
  r.check("the notice is shown only once", !(await d.js(notice)));
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
