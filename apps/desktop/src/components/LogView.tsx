import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { LogLine } from "../lib/ipc";

const levelColor: Record<string, string> = {
  WARN: "text-warning",
  ERROR: "text-danger",
  FATAL: "text-danger",
  DEBUG: "text-fg-faint",
};

function lineClass(l: LogLine | string): string {
  if (typeof l === "string") {
    return /\bERROR\b/.test(l) ? "text-danger" : /\bWARN\b/.test(l) ? "text-warning" : "text-fg-muted";
  }
  return levelColor[l.level ?? ""] ?? (l.stream === "stderr" ? "text-warning" : "text-fg-muted");
}

/**
 * Virtualized log: only visible lines are in the DOM, so thousands of lines
 * stay cheap. Follows new output while scrolled to the bottom.
 */
export function LogView({ lines, empty, label }: { lines: LogLine[] | string[]; empty: string; label: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const v = useVirtualizer({
    count: lines.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 19,
    overscan: 20,
  });

  useEffect(() => {
    if (stick.current && lines.length > 0) v.scrollToIndex(lines.length - 1, { align: "end" });
  }, [lines.length, v]);

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
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((item) => {
            const l = lines[item.index]!;
            return (
              <div
                key={item.key}
                data-index={item.index}
                ref={v.measureElement}
                style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}
                className={`whitespace-pre-wrap break-all ${lineClass(l)}`}
              >
                {typeof l === "string" ? l : l.text}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
