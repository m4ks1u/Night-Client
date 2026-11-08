import { useState } from "react";
import { Clock, Download, Loader2, LogIn, Play, Square, X } from "lucide-react";
import { Button, type ButtonProps } from "./Button";
import { ProgressBar } from "./ProgressBar";
import { InlineError } from "./ui";
import { api, toCmdError, type CmdError, type Instance } from "../lib/ipc";
import { playInstance } from "../lib/actions";
import { PHASE_LABEL, formatBytes, fraction } from "../lib/format";
import { useApp, useVersionJob } from "../stores/app";

/** Progress line for a version's active download job. */
export function JobProgress({ versionId, className = "w-72" }: { versionId: string; className?: string }) {
  const job = useVersionJob(versionId);
  const p = useApp((s) => (job.state === "active" ? s.progress : null));
  if (job.state === "queued") {
    return <p className="flex items-center gap-1.5 text-xs text-fg-muted"><Clock size={12} /> Waiting in the download queue</p>;
  }
  if (job.state !== "active") return null;
  return (
    <div className={`flex flex-col gap-1.5 ${className}`}>
      <div className="flex justify-between gap-3 text-xs">
        <span className="text-fg-muted">{p ? PHASE_LABEL[p.phase] : "Preparing…"}</span>
        {p && p.totalBytes > 0 && (
          <span className="tabular-nums text-fg-faint">
            {formatBytes(p.doneBytes)} / {formatBytes(p.totalBytes)}
          </span>
        )}
      </div>
      <ProgressBar value={p && p.totalFiles > 0 ? fraction(p) : null} label={`Downloading ${versionId}`} />
    </div>
  );
}

/** Primary action for an instance: sign in / install+play / stop / cancel, with progress and errors. */
export function PlayButton({ instance, size = "md", showProgress = true }: { instance: Instance; size?: ButtonProps["size"]; showProgress?: boolean }) {
  const account = useApp((s) => s.account);
  const running = useApp((s) => instance.id in s.running);
  const installError = useApp((s) => s.installErrors[instance.gameVersion]);
  const job = useVersionJob(instance.gameVersion);
  const [error, setError] = useState<CmdError | null>(null);
  const [starting, setStarting] = useState(false);

  async function play() {
    setError(null);
    setStarting(true);
    setError(await playInstance(instance));
    setStarting(false);
  }

  let button;
  if (running) {
    button = (
      <Button variant="danger" size={size} icon={<Square size={13} fill="currentColor" />} onClick={() => void api.stopInstance(instance.id).catch((e) => setError(toCmdError(e)))}>
        Stop
      </Button>
    );
  } else if (job.jobId !== null) {
    button = (
      <Button variant="danger" size={size} icon={<X size={14} />} onClick={() => void api.cancelDownload(job.jobId!).catch(() => {})}>
        {job.state === "queued" ? "Remove from queue" : "Cancel"}
      </Button>
    );
  } else if (!account || account.status === "expired") {
    button = (
      <Button variant="secondary" size={size} icon={<LogIn size={14} />} onClick={() => void playInstance(instance)}>
        {account ? "Sign in again" : "Sign in to play"}
      </Button>
    );
  } else {
    button = (
      <Button
        variant="primary"
        size={size}
        disabled={starting}
        icon={starting ? <Loader2 size={14} className="animate-spin" /> : instance.installed ? <Play size={14} fill="currentColor" /> : <Download size={14} />}
        onClick={() => void play()}
      >
        {instance.installed ? "Play" : "Install & play"}
      </Button>
    );
  }

  const shownError = error ?? (installError && job.state === "idle" ? installError : null);
  return (
    <div className="flex flex-col items-end gap-2">
      <div>{button}</div>
      {showProgress && <JobProgress versionId={instance.gameVersion} />}
      {showProgress && shownError && (
        <div className="max-w-md">
          <InlineError error={shownError} />
        </div>
      )}
    </div>
  );
}
