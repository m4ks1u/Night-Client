import { useEffect, useState } from "react";
import { FolderInput, FolderOpen, Undo2 } from "lucide-react";
import { Button } from "./Button";
import { ProgressBar } from "./ProgressBar";
import { Card, Dialog, InlineError, Skeleton } from "./ui";
import { api, events, toCmdError, type CmdError, type DataFolder } from "../lib/ipc";
import { formatBytes } from "../lib/format";

/** The folder that will hold the data when the user picks `picked`. */
export function targetFor(picked: string): string {
  const base = picked.replace(/[\\/]+$/, "");
  return /[\\/]Night Client$/i.test(base) ? base : `${base}\\Night Client`;
}

type Pending = { target: string | null; label: string };

/** Settings → Storage: where the game data lives, and moving it. */
export function DataFolderCard() {
  const [info, setInfo] = useState<DataFolder | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [moving, setMoving] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);

  useEffect(() => {
    api.dataFolderInfo().then(setInfo, (e) => setError(toCmdError(e)));
  }, []);

  async function choose() {
    setError(null);
    try {
      const picked = await api.pickFolder();
      if (picked) setPending({ target: targetFor(picked), label: targetFor(picked) });
    } catch (e) {
      setError(toCmdError(e));
    }
  }

  async function move(p: Pending) {
    setMoving(true);
    setProgress(null);
    const off = await events.relocateProgress((e) => setProgress(e.totalBytes > 0 ? e.doneBytes / e.totalBytes : null));
    try {
      // On success the app restarts, so this call does not return.
      await api.relocateData(p.target);
    } catch (e) {
      setError(toCmdError(e));
      setMoving(false);
      setPending(null);
    } finally {
      off();
    }
  }

  return (
    <Card title="Storage">
      {!info ? (
        error ? <InlineError error={error} /> : <Skeleton className="h-12" />
      ) : (
        <div className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <p className="text-[13px] font-medium">Game data folder</p>
              <p className="truncate font-mono text-xs text-fg-faint" title={info.path}>
                {info.path}
              </p>
              <p className="text-xs text-fg-muted">
                {formatBytes(info.sizeBytes)} of versions, libraries, assets, Java runtimes and instances
                {info.isDefault ? " (default location)" : ""}.
              </p>
            </div>
            <Button size="sm" icon={<FolderOpen size={13} />} onClick={() => void api.openDataDir()}>
              Open
            </Button>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" icon={<FolderInput size={13} />} onClick={() => void choose()}>
              Move to another folder…
            </Button>
            {!info.isDefault && (
              <Button size="sm" icon={<Undo2 size={13} />} onClick={() => setPending({ target: null, label: info.defaultPath })}>
                Move back to the default folder
              </Button>
            )}
          </div>
          {error && <InlineError error={error} />}
        </div>
      )}

      <Dialog
        open={pending !== null}
        onClose={() => !moving && setPending(null)}
        title={moving ? "Moving game data…" : "Move game data?"}
        footer={
          !moving && (
            <>
              <Button onClick={() => setPending(null)}>Cancel</Button>
              <Button variant="primary" onClick={() => pending && void move(pending)}>
                Move and restart
              </Button>
            </>
          )
        }
      >
        {pending && info && (
          <div className="flex flex-col gap-3 text-[13px] text-fg-muted">
            <p>
              <span className="text-fg">{formatBytes(info.sizeBytes)}</span> will move to
              <span className="mt-1 block break-all font-mono text-xs text-fg">{pending.label}</span>
            </p>
            {moving ? (
              <ProgressBar value={progress} label="Moving game data" />
            ) : (
              <ul className="list-disc space-y-1 pl-5 text-xs">
                <li>Close running games and wait for downloads to finish first.</li>
                <li>On the same drive this is instant; to another drive it copies, then deletes the old files.</li>
                <li>If anything fails, everything is put back. Night Client restarts when the move is done.</li>
                <li>Your accounts, settings and logs stay where they are.</li>
              </ul>
            )}
          </div>
        )}
      </Dialog>
    </Card>
  );
}
