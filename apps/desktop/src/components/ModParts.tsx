import { useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, Loader2, Package } from "lucide-react";
import { Button } from "./Button";
import { Dialog, InlineError } from "./ui";
import { api, toCmdError, type CmdError, type InstallPlan } from "../lib/ipc";
import { formatBytes } from "../lib/format";

/** Project icon from Modrinth's CDN (allowed by the CSP), lazy, with a fallback. */
export function ModIcon({ url, size = 40 }: { url: string | null; size?: number }) {
  const [failed, setFailed] = useState(false);
  const style = { width: size, height: size };
  if (!url || failed || !url.startsWith("https://cdn.modrinth.com/")) {
    return (
      <span style={style} className="grid shrink-0 place-items-center rounded-md border border-line bg-surface-2 text-fg-faint" aria-hidden="true">
        <Package size={Math.round(size * 0.45)} />
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      width={size}
      height={size}
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
      style={style}
      className="shrink-0 rounded-md bg-surface-2 object-cover"
    />
  );
}

/**
 * Review-then-install dialog: shows the version, required dependencies,
 * already-satisfied ones, missing ones and declared conflicts before
 * anything is written.
 */
export function InstallPlanDialog({ instanceId, projectId, title, onClose, onInstalled }: {
  instanceId: string;
  projectId: string | null;
  title: string;
  onClose: () => void;
  onInstalled: () => void;
}) {
  const [plan, setPlan] = useState<InstallPlan | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [busy, setBusy] = useState(false);
  const [ack, setAck] = useState(false);

  useEffect(() => {
    setPlan(null);
    setError(null);
    setAck(false);
    if (!projectId) return;
    api.modInstallPlan(instanceId, projectId).then(setPlan).catch((e) => setError(toCmdError(e)));
  }, [instanceId, projectId]);

  const name = (id: string) => plan?.titles[id] ?? id;
  const risky = !!plan && (plan.missing.length > 0 || plan.conflicts.length > 0);
  const total = plan?.install.reduce((n, m) => n + (m.version.files.find((f) => f.primary) ?? m.version.files[0])!.size, 0) ?? 0;

  async function install() {
    if (!plan) return;
    setBusy(true);
    setError(null);
    try {
      await api.modInstall(
        instanceId,
        plan.install.map((m) => ({ versionId: m.version.id, requiredBy: m.requiredBy })),
      );
      onInstalled();
      onClose();
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={projectId !== null}
      onClose={() => !busy && onClose()}
      title={`Install ${title}`}
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button variant={risky ? "danger" : "primary"} disabled={!plan || busy || (risky && !ack)} onClick={() => void install()} icon={busy ? <Loader2 size={14} className="animate-spin" /> : undefined}>
            {busy ? "Installing…" : risky ? "Install anyway" : plan && plan.install.length > 1 ? `Install ${plan.install.length} mods` : "Install"}
          </Button>
        </>
      }
    >
      {error ? (
        <InlineError error={error} />
      ) : !plan ? (
        <p className="flex items-center gap-2 py-3"><Loader2 size={14} className="animate-spin" /> Checking versions and dependencies…</p>
      ) : (
        <div className="flex flex-col gap-3">
          <ul className="flex flex-col gap-1.5">
            {plan.install.map((m) => (
              <li key={m.version.id} className="flex items-baseline justify-between gap-3 rounded-md bg-surface-3/60 px-3 py-2">
                <span className="min-w-0">
                  <span className="block truncate text-[13px] font-medium text-fg">{name(m.version.project_id)}</span>
                  <span className="block truncate text-xs">
                    {m.version.version_number}
                    {m.version.version_type !== "release" && <span className="ml-1 text-warning">({m.version.version_type})</span>}
                    {m.requiredBy && ` · required by ${name(m.requiredBy)}`}
                  </span>
                </span>
              </li>
            ))}
          </ul>
          <p className="text-xs">{formatBytes(total)} from cdn.modrinth.com, each file verified with SHA-512.</p>
          {plan.alreadyInstalled.length > 0 && (
            <p className="flex items-start gap-1.5 text-xs text-success">
              <CheckCircle2 size={13} className="mt-0.5 shrink-0" /> Already installed: {plan.alreadyInstalled.map(name).join(", ")}
            </p>
          )}
          {plan.missing.length > 0 && (
            <p className="flex items-start gap-1.5 text-xs text-warning">
              <AlertTriangle size={13} className="mt-0.5 shrink-0" />
              Required but not available for this version: {plan.missing.map(name).join(", ")}. The mod may not start.
            </p>
          )}
          {plan.conflicts.length > 0 && (
            <p className="flex items-start gap-1.5 text-xs text-danger">
              <AlertTriangle size={13} className="mt-0.5 shrink-0" />
              Declared incompatible with: {plan.conflicts.map(name).join(", ")}.
            </p>
          )}
          {risky && (
            <label className="flex items-center gap-2 text-xs text-fg">
              <input type="checkbox" className="accent-accent" checked={ack} onChange={(e) => setAck(e.target.checked)} />I understand this may not work.
            </label>
          )}
          <p className="text-2xs text-fg-faint">Only declared dependencies and conflicts can be checked. Some mods conflict in ways no launcher can detect in advance.</p>
        </div>
      )}
    </Dialog>
  );
}
