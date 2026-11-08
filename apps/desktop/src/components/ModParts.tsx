import { useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, Loader2, Package } from "lucide-react";
import { Button } from "./Button";
import { Dialog, InlineError } from "./ui";
import { api, toCmdError, type CmdError, type ContentKind, type InstallPlan, type PlannedMod } from "../lib/ipc";
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
 * Everything to install: the main plan, then each chosen optional plan,
 * each project once (an option's dependency may already be planned).
 */
export function itemsToInstall(plan: InstallPlan, chosen: ReadonlySet<string>): PlannedMod[] {
  const out: PlannedMod[] = [];
  const seen = new Set<string>();
  const add = (m: PlannedMod) => {
    if (seen.has(m.version.project_id)) return;
    seen.add(m.version.project_id);
    out.push(m);
  };
  plan.install.forEach(add);
  for (const o of plan.options) {
    if (o.install[0] && chosen.has(o.install[0].version.project_id)) o.install.forEach(add);
  }
  return out;
}

/**
 * Review-then-install dialog: shows the version, required dependencies,
 * already-satisfied ones, missing ones, declared conflicts and optional
 * extras (off by default) before anything is written.
 */
export function InstallPlanDialog({ instanceId, projectId, title, kind = "mod", onClose, onInstalled }: {
  instanceId: string;
  kind?: ContentKind;
  projectId: string | null;
  title: string;
  onClose: () => void;
  onInstalled: () => void;
}) {
  const [plan, setPlan] = useState<InstallPlan | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [busy, setBusy] = useState(false);
  const [ack, setAck] = useState(false);
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    setPlan(null);
    setError(null);
    setAck(false);
    setChosen(new Set());
    if (!projectId) return;
    api.modInstallPlan(instanceId, projectId, kind).then(setPlan).catch((e) => setError(toCmdError(e)));
  }, [instanceId, projectId, kind]);

  const name = (id: string) => plan?.titles[id] ?? id;
  const picked = plan ? plan.options.filter((o) => o.install[0] && chosen.has(o.install[0].version.project_id)) : [];
  const risky = !!plan && [plan, ...picked].some((p) => p.missing.length > 0 || p.conflicts.length > 0);
  const items = plan ? itemsToInstall(plan, chosen) : [];
  const total = items.reduce((n, m) => n + (m.version.files.find((f) => f.primary) ?? m.version.files[0])!.size, 0);

  async function install() {
    if (!plan) return;
    setBusy(true);
    setError(null);
    try {
      await api.modInstall(
        instanceId,
        items.map((m) => ({ versionId: m.version.id, requiredBy: m.requiredBy })),
        kind,
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
            {busy ? "Installing…" : risky ? "Install anyway" : items.length > 1 ? `Install ${items.length} mods` : "Install"}
          </Button>
        </>
      }
    >
      {error ? (
        <InlineError error={error} />
      ) : !plan ? (
        <p className="flex items-center gap-2 py-3"><Loader2 size={14} className="animate-spin" /> {kind === "mod" ? "Checking versions and dependencies…" : "Checking versions…"}</p>
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
          {plan.options.length > 0 && (
            <fieldset className="flex flex-col gap-1.5">
              <legend className="mb-1 text-xs font-medium text-fg">Optional extras</legend>
              {plan.options.map((o) => {
                const head = o.install[0]!;
                const pid = head.version.project_id;
                const extra = o.install.length - 1;
                const notes = [
                  extra > 0 ? `adds ${extra} required ${extra === 1 ? "mod" : "mods"}` : null,
                  o.missing.length > 0 ? `needs ${o.missing.map(name).join(", ")}, which isn't available` : null,
                  o.conflicts.length > 0 ? `declared incompatible with ${o.conflicts.map(name).join(", ")}` : null,
                ].filter(Boolean);
                return (
                  <label key={pid} className="flex items-start gap-2 rounded-md border border-line px-3 py-2 text-xs">
                    <input
                      type="checkbox"
                      className="mt-0.5 accent-accent"
                      checked={chosen.has(pid)}
                      onChange={(e) => {
                        const next = new Set(chosen);
                        if (e.target.checked) next.add(pid);
                        else next.delete(pid);
                        setChosen(next);
                        setAck(false);
                      }}
                    />
                    <span className="min-w-0">
                      <span className="block truncate text-[13px] font-medium text-fg">{name(pid)}</span>
                      <span className="block truncate">
                        {head.version.version_number}
                        {head.version.version_type !== "release" && <span className="ml-1 text-warning">({head.version.version_type})</span>}
                        {notes.length > 0 && ` · ${notes.join(" · ")}`}
                      </span>
                    </span>
                  </label>
                );
              })}
            </fieldset>
          )}
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
          {kind === "mod" && (
            <p className="text-2xs text-fg-faint">Only declared dependencies and conflicts can be checked. Some mods conflict in ways no launcher can detect in advance.</p>
          )}
        </div>
      )}
    </Dialog>
  );
}
