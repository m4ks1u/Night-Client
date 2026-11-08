import { useCallback, useEffect, useState } from "react";
import { Check, Copy, FileArchive, FolderOpen, RefreshCw } from "lucide-react";
import { Button } from "../components/Button";
import { LogView } from "../components/LogView";
import { Card, ErrorState, InlineError, PageHeader, Skeleton } from "../components/ui";
import { api, toCmdError, type CmdError, type Diagnostics as D } from "../lib/ipc";
import { formatMemory } from "../lib/format";

type UAData = { getHighEntropyValues(h: string[]): Promise<{ fullVersionList?: { brand: string; version: string }[] }> };

/** Full WebView2 version; the user-agent string is frozen at "157.0.0.0". */
async function webview2Version(): Promise<string> {
  const ua = (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
  try {
    const v = await ua?.getHighEntropyValues(["fullVersionList"]);
    const edge = v?.fullVersionList?.find((b) => /Edge|WebView/i.test(b.brand));
    if (edge) return edge.version;
  } catch {
    /* fall through */
  }
  return /Edg\/([\d.]+)/.exec(navigator.userAgent)?.[1] ?? "unknown";
}

export function Diagnostics() {
  const [data, setData] = useState<D | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [logs, setLogs] = useState<string[] | null>(null);
  const [copied, setCopied] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const [exportError, setExportError] = useState<CmdError | null>(null);

  async function exportBundle() {
    setExportError(null);
    try {
      const path = await api.exportDiagnostics();
      if (path) setExported(path);
    } catch (e) {
      setExportError(toCmdError(e));
    }
  }
  const [webview, setWebview] = useState("…");

  const load = useCallback(async () => {
    setError(null);
    try {
      const [d, l, wv] = await Promise.all([api.diagnostics(), api.recentLogs(300), webview2Version()]);
      setData(d);
      setLogs(l);
      setWebview(wv);
    } catch (e) {
      setError(toCmdError(e));
    }
  }, []);
  useEffect(() => void load(), [load]);

  const rows: [string, string][] = data
    ? [
        ["Night Client", data.appVersion],
        ["Windows", data.os],
        ["WebView2", webview],
        ["Memory", data.totalMemoryMb ? formatMemory(Math.round(data.totalMemoryMb / 512) * 512) : "unknown"],
        ["CPU threads", String(data.cpuThreads)],
        ["Database", data.database ? `schema v${data.schemaVersion ?? "?"}` : `unavailable: ${data.startupError ?? "unknown error"}`],
        ["Instances", data.instanceCount === null ? "—" : String(data.instanceCount)],
        ["Installed versions", data.installedVersions.join(", ") || "none"],
        ["Java runtimes", data.runtimes.map((r) => `${r.component} (${r.version})`).join(", ") || "none"],
        ["Microsoft sign-in", data.signInConfigured ? "configured" : "not configured in this build"],
        ["Data folder", data.dataDir],
        ["Log folder", data.logDir],
      ]
    : [];

  async function copy() {
    const text = rows.map(([k, v]) => `${k}: ${v}`).join("\n");
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title="Diagnostics"
        subtitle="Details to include when you report a problem. Tokens are never shown or logged."
        actions={
          <>
            <Button size="sm" icon={copied ? <Check size={13} /> : <Copy size={13} />} disabled={!data} onClick={() => void copy()}>
              {copied ? "Copied" : "Copy report"}
            </Button>
            <Button size="sm" icon={<FileArchive size={13} />} onClick={() => void exportBundle()}>
              Export…
            </Button>
            <Button size="sm" icon={<RefreshCw size={13} />} onClick={() => void load()}>
              Refresh
            </Button>
          </>
        }
      />
      {error ? (
        <ErrorState title="Diagnostics could not be collected" error={error} onRetry={() => void load()} />
      ) : !data ? (
        <Skeleton className="h-72" />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-4">
          {exported && (
            <p role="status" className="text-xs text-success">
              Saved to <span className="break-all font-mono">{exported}</span>. It holds this summary, recent launcher logs and crash
              reports with tokens removed. Nothing is sent automatically.
            </p>
          )}
          {exportError && <InlineError error={exportError} />}
          <Card>
            <dl className="grid grid-cols-[160px_1fr] gap-x-6 gap-y-2 text-[13px]">
              {rows.map(([k, v]) => (
                <div key={k} className="contents">
                  <dt className="text-fg-faint">{k}</dt>
                  <dd className="min-w-0 break-words text-fg-muted select-text">{v}</dd>
                </div>
              ))}
            </dl>
          </Card>
          <section aria-label="Recent launcher log" className="flex min-h-[260px] flex-1 flex-col overflow-hidden rounded-lg border border-line bg-surface-1">
            <div className="flex h-10 shrink-0 items-center justify-between border-b border-line px-4">
              <h2 className="text-[13px] font-semibold">Launcher log</h2>
              <Button variant="ghost" size="sm" icon={<FolderOpen size={13} />} onClick={() => void api.openLogsDir()}>
                Open log folder
              </Button>
            </div>
            <div className="min-h-0 flex-1">
              <LogView lines={logs ?? []} label="Launcher log" empty="No log entries yet." />
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
