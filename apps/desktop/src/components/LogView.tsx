import { useEffect, useRef } from "react";
import { useLauncher } from "../stores/launcher";

const levelColor: Record<string, string> = {
  WARN: "text-warning",
  ERROR: "text-danger",
  FATAL: "text-danger",
  DEBUG: "text-fg-faint",
};

export function LogView() {
  const logs = useLauncher((s) => s.logs);
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [logs]);

  return (
    <div
      ref={ref}
      role="log"
      aria-live="off"
      aria-label="Game output"
      tabIndex={0}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="h-full overflow-auto px-4 py-3 font-mono text-[12px] leading-[1.55] select-text"
    >
      {logs.length === 0 ? (
        <p className="text-fg-faint">Game output appears here after launch.</p>
      ) : (
        logs.map((l, i) => (
          <div key={i} className={`whitespace-pre-wrap break-all ${levelColor[l.level ?? ""] ?? (l.stream === "stderr" ? "text-warning/90" : "text-fg-muted")}`}>
            {l.text}
          </div>
        ))
      )}
    </div>
  );
}
