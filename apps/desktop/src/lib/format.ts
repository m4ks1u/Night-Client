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
} as const;

export function formatMemory(mb: number): string {
  return mb % 1024 === 0 ? `${mb / 1024} GB` : `${(mb / 1024).toFixed(1)} GB`;
}
