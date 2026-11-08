const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Binary-prefixed size with one decimal above 1 KB, e.g. `585.9 MB`. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${v} ${UNITS[i]}` : `${v.toFixed(1)} ${UNITS[i]}`;
}

/** 0..1 completion; bytes when known, otherwise files. */
export function fraction(p: { doneBytes: number; totalBytes: number; doneFiles: number; totalFiles: number }): number {
  if (p.totalBytes > 0) return Math.min(1, p.doneBytes / p.totalBytes);
  if (p.totalFiles > 0) return Math.min(1, p.doneFiles / p.totalFiles);
  return 0;
}

export const PHASE_LABEL = {
  metadata: "Fetching metadata",
  runtime: "Installing Java runtime",
  gameFiles: "Downloading game files",
  loader: "Installing mod loader",
} as const;

export function formatMemory(mb: number): string {
  return mb % 1024 === 0 ? `${mb / 1024} GB` : `${(mb / 1024).toFixed(1)} GB`;
}

/** `2h 05m`, `12m`, `< 1m`. */
export function formatDuration(seconds: number): string {
  if (seconds < 60) return "< 1m";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return h > 0 ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}m`;
}

const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

/** "just now", "5 minutes ago", "yesterday", or a date for older times. */
export function formatRelative(unixSeconds: number | null, now = Date.now() / 1000): string {
  if (unixSeconds === null) return "Never";
  const diff = unixSeconds - now;
  const abs = Math.abs(diff);
  if (abs < 60) return "Just now";
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 7) return rtf.format(Math.round(diff / 86400), "day");
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/** Default instance name for a version, e.g. "Minecraft 1.21.1". */
export function defaultInstanceName(version: string): string {
  return `Minecraft ${version}`;
}
