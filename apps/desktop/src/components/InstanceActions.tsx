import { useState } from "react";
import { Download, Loader2, LogIn, Play, Square, X } from "lucide-react";
import { Button, type ButtonProps } from "./Button";
import { ProgressBar } from "./ProgressBar";
import { InlineError } from "./ui";
import { api, toCmdError, type CmdError, type Instance } from "../lib/ipc";
import { playInstance } from "../lib/actions";
import { PHASE_LABEL, formatBytes, fraction } from "../lib/format";
import { useApp } from "../stores/app";

/** Primary action for an instance: sign in / install+play / stop, with progress and errors. */
export function PlayButton({ instance, size = "md", showProgress = true }: { instance: Instance; size?: ButtonProps["size"]; showProgress?: boolean }) {
  const account = useApp((s) => s.account);
  const running = useApp((s) => instance.id in s.running);
  const install = useApp((s) => s.install);
  const installError = useApp((s) => s.installErrors[instance.gameVersion]);
  const [error, setError] = useState<CmdError | null>(null);
  const [starting, setStarting] = useState(false);

  const installingThis = install?.versionId === instance.gameVersion;
  const otherInstall = install !== null && !installingThis;

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
  } else if (installingThis) {
    button = (
      <Button variant="danger" size={size} icon={<X size={14} />} onClick={() => void api.cancelInstall()}>
        Cancel
      </Button>
    );
  } else if (!account) {
    button = (
      <Button variant="secondary" size={size} icon={<LogIn size={14} />} onClick={() => void playInstance(instance)}>
        Sign in to play
      </Button>
    );
  } else {
    button = (
      <Button
        variant="primary"
        size={size}
        disabled={starting || (otherInstall && !instance.installed)}
        title={otherInstall && !instance.installed ? `Waiting for ${install?.versionId} to finish installing` : undefined}
        icon={starting ? <Loader2 size={14} className="animate-spin" /> : instance.installed ? <Play size={14} fill="currentColor" /> : <Download size={14} />}
        onClick={() => void play()}
      >
        {instance.installed ? "Play" : "Install & play"}
      </Button>
    );
  }

  const p = installingThis ? install.progress : null;
  const shownError = error ?? (installError && !installingThis ? installError : null);
  return (
    <div className="flex flex-col gap-2">
      <div>{button}</div>
      {showProgress && installingThis && (
        <div className="flex w-72 flex-col gap-1.5">
          <div className="flex justify-between text-xs">
            <span className="text-fg-muted">{p ? PHASE_LABEL[p.phase] : "Preparing…"}</span>
            {p && p.totalBytes > 0 && (
              <span className="tabular-nums text-fg-faint">
                {formatBytes(p.doneBytes)} / {formatBytes(p.totalBytes)}
              </span>
            )}
          </div>
          <ProgressBar value={p && p.totalFiles > 0 ? fraction(p) : null} label={`Installing ${instance.gameVersion}`} />
        </div>
      )}
      {showProgress && shownError && <div className="max-w-md"><InlineError error={shownError} /></div>}
    </div>
  );
}
