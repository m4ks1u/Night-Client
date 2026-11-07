import { useEffect, useRef } from "react";
import type { LogLine } from "../lib/ipc";

const levelColor: Record<string, string> = {
  WARN: "text-warning",
  ERROR: "text-danger",
  FATAL: "text-danger",
  DEBUG: "text-fg-faint",
};

export function LogView({ lines, empty, label }: { lines: LogLine[] | string[]; empty: string; label: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div
      ref={ref}
      role="log"
      aria-live="off"
      aria-label={label}
      tabIndex={0}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="h-full overflow-auto px-4 py-3 font-mono text-[12px] leading-[1.55] select-text"
    >
      {lines.length === 0 ? (
        <p className="text-fg-faint">{empty}</p>
      ) : (
        lines.map((l, i) =>
          typeof l === "string" ? (
            <div key={i} className={`whitespace-pre-wrap break-all ${/\bERROR\b/.test(l) ? "text-danger" : /\bWARN\b/.test(l) ? "text-warning" : "text-fg-muted"}`}>
              {l}
            </div>
          ) : (
            <div key={i} className={`whitespace-pre-wrap break-all ${levelColor[l.level ?? ""] ?? (l.stream === "stderr" ? "text-warning/90" : "text-fg-muted")}`}>
              {l.text}
            </div>
          ),
        )
      )}
    </div>
  );
}
