import { useCallback, useEffect, useState } from "react";
import { CheckCircle2, ChevronRight, CircleSlash, Download, Trash2, X, XCircle } from "lucide-react";
import { Button } from "../components/Button";
import { JobProgress } from "../components/InstanceActions";
import { Badge, Card, Dialog, EmptyState, ErrorState, PageHeader, Skeleton } from "../components/ui";
import { api, toCmdError, type CmdError, type DownloadRecord } from "../lib/ipc";
import { formatBytes, formatDuration, formatRelative } from "../lib/format";
import { useApp } from "../stores/app";

export function Downloads() {
  const queue = useApp((s) => s.queue);
  const historyVersion = useApp((s) => s.historyVersion);
  const [history, setHistory] = useState<DownloadRecord[] | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      setHistory(await api.downloadHistory(100));
    } catch (e) {
      setError(toCmdError(e));
    }
  }, []);
  useEffect(() => void load(), [load, historyVersion]);

  const idle = !queue.active && queue.pending.length === 0;

  return (
    <div>
      <PageHeader title="Downloads" subtitle="Game files and Java runtimes are downloaded one version at a time and verified against Mojang's hashes." />

      <div className="flex flex-col gap-6">
        <section aria-labelledby="dl-active">
          <h2 id="dl-active" className="mb-3 text-[13px] font-semibold">
            In progress
          </h2>
          {idle ? (
            <p className="rounded-lg border border-line bg-surface-1 px-4 py-5 text-[13px] text-fg-muted">Nothing is downloading.</p>
          ) : (
            <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
              {queue.active && (
                <li className="flex items-center gap-4 px-4 py-4">
                  <Download size={16} className="shrink-0 text-accent" />
                  <div className="min-w-0 flex-1">
                    <p className="mb-2 text-[13px] font-medium">Minecraft {queue.active.target}</p>
                    <JobProgress versionId={queue.active.target} className="w-full" />
                  </div>
                  <Button size="sm" variant="danger" icon={<X size={13} />} onClick={() => void api.cancelDownload(queue.active!.id)}>
                    Cancel
                  </Button>
                </li>
              )}
              {queue.pending.map((j, i) => (
                <li key={j.id} className="flex items-center gap-4 px-4 py-3">
                  <span className="w-4 shrink-0 text-center text-xs tabular-nums text-fg-faint">{i + 1}</span>
                  <p className="min-w-0 flex-1 text-[13px]">
                    Minecraft {j.target} <span className="text-xs text-fg-faint">· queued {formatRelative(j.queuedAt).toLowerCase()}</span>
                  </p>
                  <Button size="sm" variant="ghost" icon={<X size={13} />} onClick={() => void api.cancelDownload(j.id)}>
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section aria-labelledby="dl-history">
          <div className="mb-3 flex items-center justify-between">
            <h2 id="dl-history" className="text-[13px] font-semibold">
              History
            </h2>
            {history && history.length > 0 && (
              <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} onClick={() => setConfirmClear(true)}>
                Clear history
              </Button>
            )}
          </div>
          {error ? (
            <ErrorState title="History could not be loaded" error={error} onRetry={() => void load()} />
          ) : !history ? (
            <Skeleton className="h-32" />
          ) : history.length === 0 ? (
            <EmptyState icon={<Download size={20} />} title="No downloads yet" body="Installs and file checks you run appear here, including which damaged files were repaired." />
          ) : (
            <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
              {history.map((r) => (
                <HistoryRow key={r.id} r={r} />
              ))}
            </ul>
          )}
        </section>
      </div>

      <Dialog
        open={confirmClear}
        onClose={() => setConfirmClear(false)}
        title="Clear download history?"
        footer={
          <>
            <Button onClick={() => setConfirmClear(false)}>Cancel</Button>
            <Button
              variant="danger"
              onClick={async () => {
                setConfirmClear(false);
                await api.clearDownloadHistory().catch((e) => setError(toCmdError(e)));
                void load();
              }}
            >
              Clear
            </Button>
          </>
        }
      >
        This only removes the list. Downloaded game files are kept.
      </Dialog>
    </div>
  );
}

function HistoryRow({ r }: { r: DownloadRecord }) {
  const [open, setOpen] = useState(false);
  const icon =
    r.status === "completed" ? <CheckCircle2 size={15} className="text-success" /> : r.status === "cancelled" ? <CircleSlash size={15} className="text-fg-faint" /> : <XCircle size={15} className="text-danger" />;
  const summary =
    r.status === "failed"
      ? "Failed"
      : r.status === "cancelled"
        ? "Cancelled"
        : r.repair
          ? r.filesDownloaded === 0
            ? `All ${r.filesVerified.toLocaleString()} files verified`
            : `${r.filesDownloaded.toLocaleString()} damaged or missing ${r.filesDownloaded === 1 ? "file" : "files"} repaired`
          : `Installed · ${formatBytes(r.bytesDownloaded)}`;
  const hasDetails = (r.repair && r.repaired.length > 0) || !!r.error;
  return (
    <li>
      <button
        type="button"
        disabled={!hasDetails}
        aria-expanded={hasDetails ? open : undefined}
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-3 px-4 py-3 text-left enabled:hover:bg-surface-2/60"
      >
        {icon}
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2 text-[13px] font-medium">
            Minecraft {r.target}
            {r.repair && <Badge>Verify</Badge>}
          </span>
          <span className={`text-xs ${r.status === "failed" ? "text-danger" : "text-fg-faint"}`}>
            {summary} · {formatDuration(Math.max(0, r.finishedAt - r.startedAt))} · {formatRelative(r.finishedAt)}
          </span>
        </span>
        {hasDetails && <ChevronRight size={14} className={`text-fg-faint transition-transform duration-(--duration-fast) ${open ? "rotate-90" : ""}`} />}
      </button>
      {open && (
        <div className="border-t border-line bg-bg/40 px-4 py-3">
          {r.error && <p className="mb-2 text-xs text-danger select-text">{r.error}</p>}
          {r.repaired.length > 0 && (
            <Card className="p-3">
              <ul className="max-h-48 overflow-auto font-mono text-[11px] leading-relaxed text-fg-muted select-text">
                {r.repaired.map((p) => (
                  <li key={p}>{p}</li>
                ))}
              </ul>
              {r.filesDownloaded > r.repaired.length && (
                <p className="mt-2 text-2xs text-fg-faint">…and {r.filesDownloaded - r.repaired.length} more.</p>
              )}
            </Card>
          )}
        </div>
      )}
    </li>
  );
}
