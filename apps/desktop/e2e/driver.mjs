// Minimal driver for end-to-end tests of the desktop app over the WebView2
// DevTools protocol. Works only with DEBUG builds, which open the port when
// NIGHT_CLIENT_DEVTOOLS_PORT is set (release builds never do).

import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Errors are persisted in sessionStorage so they survive reloads. */
const ERROR_HOOK = `(window.__err=JSON.parse(sessionStorage.getItem("__err")||"[]"),
  window.addEventListener("error",e=>{window.__err.push(String(e.message));sessionStorage.setItem("__err",JSON.stringify(window.__err))}),
  window.addEventListener("unhandledrejection",e=>{window.__err.push(String(e.reason?.message??JSON.stringify(e.reason)));sessionStorage.setItem("__err",JSON.stringify(window.__err))}),true)`;

export async function launch(exe, { port = 9350, outDir = "." } = {}) {
  const app = spawn(exe, [], { env: { ...process.env, NIGHT_CLIENT_DEVTOOLS_PORT: String(port) }, stdio: "ignore" });
  let target;
  for (let i = 0; i < 80 && !target; i++) {
    await sleep(250);
    try {
      target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === "page");
    } catch {}
  }
  if (!target) throw new Error("no DevTools target (is this a debug build?)");
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const pending = new Map();
  /** Uncaught exceptions and console errors reported by DevTools (including at startup). */
  const consoleErrors = [];
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    if (d.id && pending.has(d.id)) {
      pending.get(d.id)(d);
      pending.delete(d.id);
    } else if (d.method === "Runtime.exceptionThrown") {
      consoleErrors.push(d.params.exceptionDetails?.exception?.description ?? d.params.exceptionDetails?.text);
    } else if (d.method === "Runtime.consoleAPICalled" && d.params.type === "error") {
      consoleErrors.push(d.params.args.map((a) => a.value ?? a.description).join(" "));
    }
  };
  const send = (method, params = {}) =>
    new Promise((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  const js = async (expr) => {
    const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 400));
    return r.result?.result?.value;
  };
  await send("Runtime.enable");
  const d = {
    app,
    consoleErrors,
    send,
    js,
    /** Invoke a Tauri command directly (bypassing the UI). */
    inv: (cmd, args = {}) =>
      js(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then(v=>({ok:true,v}),e=>({ok:false,e}))`),
    shot: async (name) =>
      writeFileSync(`${outDir}/${name}.png`, Buffer.from((await send("Page.captureScreenshot", { format: "png" })).result.data, "base64")),
    /** Click the first visible, enabled element in `sel` whose text, aria-label or <label> matches `re`. */
    click: (re, sel = "button") =>
      js(`(()=>{const name=b=>(b.textContent+' '+(b.getAttribute('aria-label')||'')+' '+[...(b.labels||[])].map(l=>l.textContent).join(' ')).trim();const b=[...document.querySelectorAll(${JSON.stringify(sel)})].find(b=>b.offsetParent!==null&&${re}.test(name(b))&&!b.disabled);if(b){b.click();return true}return false})()`),
    waitFor: async (pred, ms = 15000) => {
      const t = Date.now();
      while (Date.now() - t < ms) {
        if (await js(pred)) return true;
        await sleep(150);
      }
      return false;
    },
    setValue: (sel, value) =>
      js(`(()=>{const el=document.querySelector(${JSON.stringify(sel)});const P=el.tagName==='SELECT'?HTMLSelectElement.prototype:el.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(P,'value').set.call(el,${JSON.stringify(value)});el.dispatchEvent(new Event(el.tagName==='SELECT'?'change':'input',{bubbles:true}));return el.value})()`),
    key: async (key, modifiers = 0) => {
      // modifiers: 1 Alt, 2 Ctrl, 4 Meta, 8 Shift
      const named = { Enter: [13, "Enter"], Escape: [27, "Escape"], ",": [188, "Comma"], ArrowDown: [40, "ArrowDown"], ArrowUp: [38, "ArrowUp"] };
      const [vk, code] = named[key] ?? [key.toUpperCase().charCodeAt(0), `Key${key.toUpperCase()}`];
      const base = { key, code, modifiers, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk };
      await send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
      await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    },
    hookErrors: () => js(ERROR_HOOK),
    errors: async () => (await js("window.__err")) ?? [],
    reload: async () => {
      await js("window.location.reload()");
      await sleep(2500);
      await js(ERROR_HOOK);
    },
    close: () => app.kill(),
  };
  await sleep(2000);
  // Key events need a focused page even when the window is in the background.
  await send("Emulation.setFocusEmulationEnabled", { enabled: true });
  try {
    await d.hookErrors();
  } catch (e) {
    app.kill();
    // Debug builds load the UI from the Vite dev server; without it WebView2 shows an error page.
    throw new Error(`the app page did not load. Is the Vite dev server running (pnpm dev)? ${e.message.slice(0, 120)}`);
  }
  return d;
}

export function reporter() {
  const results = [];
  return {
    check: (name, ok, extra = "") => results.push(`${ok ? "PASS" : "FAIL"} ${name}${extra ? " " + extra : ""}`),
    print: () => {
      console.log(results.join("\n"));
      return results.some((r) => r.startsWith("FAIL")) ? 1 : 0;
    },
  };
}
