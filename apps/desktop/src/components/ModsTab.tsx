import { useCallback, useEffect, useState } from "react";
import { ArrowUpCircle, Compass, FilePlus2, Loader2, Package, RefreshCw, ShieldAlert, Trash2 } from "lucide-react";
import { Button } from "./Button";
import { ModIcon } from "./ModParts";
import { Badge, Dialog, EmptyState, ErrorState, InlineError, Skeleton, Toggle } from "./ui";
import { api, toCmdError, type CmdError, type Instance, type ModRecord, type ModUpdate } from "../lib/ipc";
import { formatBytes } from "../lib/format";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

export function ModsTab({ instance }: { instance: Instance }) {
  const running = useApp((s) => instance.id in s.running);
  const [mods, setMods] = useState<ModRecord[] | null>(null);
  const [loadError, setLoadError] = useState<CmdError | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [notice, setNotice] = useState<string[]>([]);
  const [updates, setUpdates] = useState<ModUpdate[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [removing, setRemoving] = useState<ModRecord | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      setMods(await api.listMods(instance.id));
    } catch (e) {
      setLoadError(toCmdError(e));
    }
  }, [instance.id]);
  useEffect(() => void load(), [load]);

  async function run(label: string, fn: () => Promise<unknown>) {
    setBusy(label);
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(null);
    }
  }

  const updateFor = (f: string) => updates?.find((u) => u.fileName === f);
  // `requiredBy` is stored on the dependency: Sodium's row says Iris pulled it in.
  // So removing Sodium warns about the mod named in its own `requiredBy`.
  const dependents = (m: ModRecord) => (mods ?? []).filter((x) => m.requiredBy && x.projectId === m.requiredBy && x.fileName !== m.fileName);

  if (loadError) return <ErrorState title="Mods could not be listed" error={loadError} onRetry={() => void load()} />;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" icon={<Compass size={13} />} onClick={() => navigate({ name: "discover", instanceId: instance.id })}>
          Browse Modrinth
        </Button>
        <Button
          size="sm"
          icon={<FilePlus2 size={13} />}
          disabled={running || busy !== null}
          onClick={() =>
            void run("import", async () => {
              const r = await api.importLocalMod(instance.id);
              setNotice(r?.warnings ?? []);
            })
          }
        >
          Add file
        </Button>
        <Button
          size="sm"
          icon={busy === "check" ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
          disabled={busy !== null || !mods?.some((m) => m.source === "modrinth")}
          onClick={() => void run("check", async () => setUpdates(await api.checkModUpdates(instance.id)))}
        >
          Check for updates
        </Button>
        {updates && updates.length > 0 && (
          <Button
            size="sm"
            variant="primary"
            icon={busy === "update" ? <Loader2 size={13} className="animate-spin" /> : <ArrowUpCircle size={13} />}
            disabled={running || busy !== null}
            onClick={() =>
              void run("update", async () => {
                await api.updateMods(instance.id, updates.map((u) => u.fileName));
                setUpdates([]);
              })
            }
          >
            Update all ({updates.length})
          </Button>
        )}
        {updates && updates.length === 0 && busy === null && <span className="text-xs text-success">All mods are up to date.</span>}
        {running && <span className="ml-auto text-xs text-fg-faint">Close the game to change mods.</span>}
      </div>

      {error && <InlineError error={error} />}
      {notice.length > 0 && (
        <div role="status" className="flex items-start gap-2 rounded-md border border-warning/30 px-3 py-2 text-xs text-warning">
          <ShieldAlert size={14} className="mt-0.5 shrink-0" />
          <div>
            {notice.map((n) => (
              <p key={n}>{n}</p>
            ))}
          </div>
        </div>
      )}

      {!mods ? (
        <Skeleton className="h-40" />
      ) : mods.length === 0 ? (
        <EmptyState icon={<Package size={20} />} title="No mods yet" body="Find mods on Modrinth, or add a .jar file you already have." />
      ) : (
        <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1" aria-label="Installed mods">
          {mods.map((m) => {
            const up = updateFor(m.fileName);
            return (
              <li key={m.fileName} className={`flex items-center gap-3 px-4 py-3 ${m.enabled ? "" : "opacity-60"}`}>
                <ModIcon url={m.iconUrl} size={36} />
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2 text-[13px] font-medium">
                    <span className="truncate">{m.title}</span>
                    {m.source === "local" && <Badge tone="warning">Unverified file</Badge>}
                    {m.requiredBy && <Badge>Dependency</Badge>}
                    {up && <Badge tone="accent">Update: {up.versionNumber}</Badge>}
                  </p>
                  <p className="truncate text-xs text-fg-faint" title={m.fileName}>
                    {m.versionNumber ?? m.fileName} · {formatBytes(m.size)}
                  </p>
                </div>
                <div className="w-28">
                  <Toggle
                    id={`mod-${m.fileName}`}
                    checked={m.enabled}
                    disabled={running || busy !== null}
                    onChange={(v) => void run("toggle", () => api.setModEnabled(instance.id, m.fileName, v))}
                    label={m.enabled ? "Enabled" : "Disabled"}
                  />
                </div>
                <Button size="sm" variant="ghost" aria-label={`Remove ${m.title}`} icon={<Trash2 size={13} />} disabled={running || busy !== null} onClick={() => setRemoving(m)} />
              </li>
            );
          })}
        </ul>
      )}

      <Dialog
        open={removing !== null}
        onClose={() => setRemoving(null)}
        title={`Remove ${removing?.title ?? ""}?`}
        footer={
          <>
            <Button onClick={() => setRemoving(null)}>Cancel</Button>
            <Button
              variant="danger"
              onClick={() => {
                const m = removing;
                setRemoving(null);
                if (m) void run("remove", () => api.removeMod(instance.id, m.fileName));
              }}
            >
              Remove
            </Button>
          </>
        }
      >
        The file <code className="font-mono text-fg">{removing?.fileName}</code> is deleted from this instance's mods folder.
        {removing && dependents(removing).length > 0 && (
          <span className="mt-2 block text-warning">
            Required by: {dependents(removing).map((d) => d.title).join(", ")}. They may stop working.
          </span>
        )}
      </Dialog>
    </div>
  );
}
