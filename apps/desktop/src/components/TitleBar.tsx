import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";

const ctl =
  "grid h-full w-11 place-items-center text-fg-muted transition-colors duration-(--duration-fast) hover:bg-surface-3 hover:text-fg focus-visible:outline-offset-[-2px]";

export function TitleBar() {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let off: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    void win.onResized(sync).then((u) => (off = u));
    return () => off?.();
  }, [win]);

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 items-center justify-between border-b border-line bg-bg pl-3"
    >
      <div data-tauri-drag-region className="flex items-center gap-2 text-[13px] font-medium tracking-tight text-fg">
        <Logo />
        Night Client
        <span className="rounded-sm border border-line px-1.5 text-2xs font-medium text-fg-faint">Preview</span>
      </div>
      <div className="flex h-full">
        <button className={ctl} aria-label="Minimize" onClick={() => void win.minimize()}>
          <Minus size={15} strokeWidth={1.75} />
        </button>
        <button className={ctl} aria-label={maximized ? "Restore" : "Maximize"} onClick={() => void win.toggleMaximize()}>
          {maximized ? <Copy size={13} strokeWidth={1.75} /> : <Square size={12} strokeWidth={1.75} />}
        </button>
        <button className={`${ctl} hover:bg-[#c42b1c] hover:text-white`} aria-label="Close" onClick={() => void win.close()}>
          <X size={16} strokeWidth={1.75} />
        </button>
      </div>
    </header>
  );
}

function Logo() {
  return (
    <svg width="16" height="16" viewBox="0 0 1024 1024" aria-hidden="true">
      <path d="M600 214a312 312 0 1 0 210 532A256 256 0 1 1 600 214Z" fill="var(--color-accent)" />
      <path d="M700 282l18 50 50 18-50 18-18 50-18-50-50-18 50-18z" fill="#C9C2FF" />
    </svg>
  );
}
