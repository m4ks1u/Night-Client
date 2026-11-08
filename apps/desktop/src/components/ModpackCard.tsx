import { useEffect, useState } from "react";
import { Button } from "./Button";
import { ModIcon } from "./ModParts";
import { ProgressBar } from "./ProgressBar";
import { Card, Dialog, InlineError } from "./ui";
import { api, events, toCmdError, type CmdError, type Instance, type LatestPack, type ModpackInfo, type ModpackProgress, type UpdateReport } from "../lib/ipc";
import { formatBytes } from "../lib/format";
import { useApp } from "../stores/app";

type Check = { state: "idle" } | { state: "checking" } | { state: "done"; latest: LatestPack | null } | { state: "error"; error: CmdError };

/** Overview card for an instance made from a modpack: version, update check, update. */
export function ModpackCard({ instance, disabled }: { instance: Instance; disabled: boolean }) {
  const [info, setInfo] = useState<ModpackInfo | null>(null);
  const [check, setCheck] = useState<Check>({ state: "idle" });
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ModpackProgress | null>(null);
  const [result, setResult] = useState<UpdateReport | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const upsert = useApp((s) => s.upsertInstance);
  const installVersion = useApp((s) => s.installVersion);

  useEffect(() => {
    api.modpackInfo(instance.id).then(setInfo).catch(() => setInfo(null));
  }, [instance.id]);
  useEffect(() => {
    if (!busy) return;
    const un = events.modpackProgress((p) => p.token === instance.id && setProgress(p));
    return () => void un.then((f) => f());
  }, [busy, instance.id]);

  if (!info) return null;

  async function runCheck() {
    setCheck({ state: "checking" });
    setResult(null);
    try {
      setCheck({ state: "done", latest: await api.modpackCheckUpdate(instance.id) });
    } catch (e) {
      setCheck({ state: "error", error: toCmdError(e) });
    }
  }

  async function runUpdate(latest: LatestPack) {
    setConfirm(false);
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      const r = await api.modpackUpdate(instance.id, latest.versionId);
      upsert(r.instance);
      setResult(r.report);
      setCheck({ state: "idle" });
      setInfo(await api.modpackInfo(instance.id));
      if (!r.instance.installed && r.instance.installKey) void installVersion(r.instance.installKey);
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(false);
    }
  }

  const latest = check.state === "done" ? check.latest : null;
  const mcChanges = latest !== null && latest.gameVersions.length > 0 && !latest.gameVersions.includes(instance.gameVersion);
  const pct = progress && progress.totalBytes > 0 ? progress.doneBytes / progress.totalBytes : null;
  return (
    <Card
      title="Modpack"
      actions={
        info.projectId && !busy ? (
          <Button size="sm" disabled={disabled || check.state === "checking"} onClick={() => void runCheck()}>
            {check.state === "checking" ? "Checking…" : "Check for updates"}
          </Button>
        ) : null
      }
    >
      <div className="flex flex-col gap-3 text-[13px]">
        <div className="flex items-center gap-3">
          <ModIcon url={info.iconUrl} size={36} />
          <div className="min-w-0">
            <p className="truncate font-medium text-fg">{info.name}</p>
            <p className="text-xs text-fg-muted">Version {info.versionNumber}</p>
          </div>
        </div>
        {!info.projectId && (
          <p className="text-xs text-fg-muted">Imported from a file that Modrinth doesn&apos;t list, so updates can&apos;t be checked here.</p>
        )}
        {check.state === "error" && <InlineError error={check.error} />}
        {check.state === "done" && !latest && (
          <p className="text-xs text-fg-muted" role="status">
            You have the newest version.
          </p>
        )}
        {latest && !busy && (
          <div className="flex items-center justify-between gap-3 rounded-md border border-line bg-surface-2 px-3 py-2" role="status">
            <p className="min-w-0 text-xs">
              <span className="text-fg">Version {latest.versionNumber}</span> is available
              {mcChanges ? ` (Minecraft ${latest.gameVersions.join(", ")})` : ""}.
            </p>
            <Button size="sm" variant="primary" disabled={disabled} onClick={() => setConfirm(true)}>
              Update
            </Button>
          </div>
        )}
        {busy && (
          <div className="flex flex-col gap-1.5" aria-live="polite">
            <ProgressBar value={pct} label="Modpack update" />
            <div className="flex items-center justify-between gap-3">
              <p className="text-xs text-fg-muted">
                {progress
                  ? `${progress.doneFiles} of ${progress.totalFiles} files · ${formatBytes(progress.doneBytes)} of ${formatBytes(progress.totalBytes)}`
                  : "Preparing the update…"}
              </p>
              <Button size="sm" onClick={() => void api.modpackCancel(instance.id)}>
                Cancel
              </Button>
            </div>
          </div>
        )}
        {error && <InlineError error={error} />}
        {result && (
          <div className="text-xs text-fg-muted" role="status">
            <p>
              Updated: {result.added} added, {result.replaced} replaced, {result.removed} removed, {result.unchanged} unchanged.
            </p>
            {result.kept.length > 0 && (
              <details className="mt-1">
                <summary className="cursor-pointer">{result.kept.length} files kept as you had them</summary>
                <ul className="mt-1 max-h-40 overflow-auto font-mono text-[11px]">
                  {result.kept.map((k) => (
                    <li key={k}>{k}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </div>
      <Dialog
        open={confirm && latest !== null}
        onClose={() => setConfirm(false)}
        title={`Update ${info.name}?`}
        footer={
          <>
            <Button onClick={() => setConfirm(false)}>Cancel</Button>
            <Button variant="primary" onClick={() => latest && void runUpdate(latest)}>
              Update to {latest?.versionNumber}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-2">
          <p>
            Files the pack installed that you haven&apos;t touched are replaced or removed. Files you changed, deleted or added yourself are kept, and mods
            you disabled stay disabled.
          </p>
          {mcChanges && (
            <p className="text-fg">
              This version is for Minecraft {latest?.gameVersions.join(", ")}. Back up your worlds first: a world opened in a newer version can&apos;t go
              back.
            </p>
          )}
        </div>
      </Dialog>
    </Card>
  );
}
