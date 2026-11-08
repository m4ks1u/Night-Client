import { useId, useState } from "react";
import { ArrowUpCircle, Loader2, RefreshCw } from "lucide-react";
import { Button } from "./Button";
import { ProgressBar } from "./ProgressBar";
import { Card, InlineError, Toggle } from "./ui";
import { api, events, toCmdError, type CmdError, type UpdateInfo } from "../lib/ipc";

type Props = { autoCheck: boolean; onAutoCheck: (v: boolean) => void };

/** Settings → Updates: check, read the notes, install. Notes are plain text, never HTML. */
export function UpdatesCard({ autoCheck, onAutoCheck }: Props) {
  const id = useId();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<CmdError | null>(null);

  async function check() {
    setBusy("check");
    setError(null);
    try {
      setInfo(await api.checkForUpdate());
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(null);
    }
  }

  async function install() {
    setBusy("install");
    setError(null);
    setProgress(null);
    const off = await events.updateProgress((p) => setProgress(p.totalBytes ? p.doneBytes / p.totalBytes : null));
    try {
      // On success the installer closes Night Client and starts the new version.
      await api.installUpdate();
    } catch (e) {
      setError(toCmdError(e));
      setBusy(null);
    } finally {
      off();
    }
  }

  const unconfigured = info !== null && !info.configured;
  return (
    <Card title="Updates">
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between gap-4">
          <p className="text-[13px] text-fg-muted">
            {info === null && "Night Client checks a signed release feed for new versions."}
            {unconfigured && "This is a development build without an update channel. Updates are only offered in official releases."}
            {info?.configured && !info.available && `You're up to date (version ${info.currentVersion}).`}
            {info?.available && `Version ${info.available.version} is available. You have ${info.currentVersion}.`}
          </p>
          <Button
            size="sm"
            icon={busy === "check" ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
            disabled={busy !== null}
            onClick={() => void check()}
          >
            Check now
          </Button>
        </div>

        {info?.available && (
          <div className="flex flex-col gap-3">
            {info.available.notes && (
              <div className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-md border border-line bg-surface-1 px-3 py-2 text-xs text-fg-muted">
                {info.available.notes}
              </div>
            )}
            {busy === "install" ? (
              <ProgressBar value={progress} label="Downloading update" />
            ) : (
              <div className="flex items-center gap-3">
                <Button variant="primary" size="sm" icon={<ArrowUpCircle size={13} />} disabled={busy !== null} onClick={() => void install()}>
                  Download and install
                </Button>
                <span className="text-xs text-fg-faint">The download is verified before it runs. Night Client restarts afterwards.</span>
              </div>
            )}
          </div>
        )}

        {error && <InlineError error={error} />}

        <Toggle
          id={id}
          checked={autoCheck}
          onChange={onAutoCheck}
          label="Check for updates automatically"
          description="Shortly after start, ask the release feed whether a newer version exists. This sends your Night Client version and Windows architecture; nothing else."
        />
      </div>
    </Card>
  );
}
