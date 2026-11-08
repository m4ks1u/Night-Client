import { useEffect, useId, useRef, useState } from "react";
import { ArrowLeft, FolderOpen, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "../components/Button";
import { PlayButton } from "../components/InstanceActions";
import { LogView } from "../components/LogView";
import { Badge, Card, Dialog, ErrorState, Field, InlineError, MemorySlider, Skeleton, Toggle, inputClass } from "../components/ui";
import { api, toCmdError, type CmdError, type Instance } from "../lib/ipc";
import { formatDuration, formatMemory, formatRelative } from "../lib/format";
import { effectiveMemory, useApp, useVersionJob } from "../stores/app";
import { InstanceJava } from "../components/JavaPicker";
import { JvmArgsCard } from "../components/JvmArgs";
import { ModsTab } from "../components/ModsTab";
import { GameFolder, LoaderCard } from "../components/InstanceSettings";
import { navigate, useRouter } from "../stores/router";

export function InstanceDetail({ id, tab = "overview" }: { id: string; tab?: "overview" | "mods" | "log" }) {
  const instances = useApp((s) => s.instances);
  const refresh = useApp((s) => s.refreshInstances);
  const back = useRouter((s) => s.back);

  if (instances.status === "error") return <ErrorState title="Instance could not be loaded" error={instances.error} onRetry={() => void refresh()} />;
  if (instances.status !== "ready") return <Skeleton className="h-48" />;
  const instance = instances.data.find((i) => i.id === id);
  if (!instance) {
    return (
      <ErrorState
        title="Instance not found"
        error={{ code: "not_found", message: "It may have been deleted." }}
        onRetry={() => navigate({ name: "instances" })}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Button variant="ghost" size="sm" className="-ml-2 mb-3 self-start" icon={<ArrowLeft size={14} />} onClick={back}>
        Back
      </Button>
      <header className="flex items-start justify-between gap-6 pb-5">
        <div className="min-w-0">
          <h1 className="truncate text-[22px] font-semibold tracking-tight">{instance.name}</h1>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-fg-muted">
            <Badge tone="accent">{instance.label}</Badge>
            {!instance.installed && <Badge tone="warning">Not installed</Badge>}
            <span>Last played {formatRelative(instance.lastPlayedAt).toLowerCase()}</span>
            {instance.playTimeSeconds > 0 && <span>· {formatDuration(instance.playTimeSeconds)} played</span>}
          </div>
        </div>
        <PlayButton instance={instance} size="lg" />
      </header>

      <div role="tablist" aria-label="Instance sections" className="mb-5 flex gap-1 border-b border-line">
        {(instance.loader === "vanilla" ? (["overview", "log"] as const) : (["overview", "mods", "log"] as const)).map((t) => (
          <button
            key={t}
            role="tab"
            type="button"
            aria-selected={tab === t}
            onClick={() => navigate({ name: "instance", id, tab: t })}
            className={`-mb-px border-b-2 px-3 pb-2.5 text-[13px] font-medium transition-colors ${tab === t ? "border-accent text-fg" : "border-transparent text-fg-muted hover:text-fg"}`}
          >
            {t === "overview" ? "Overview" : t === "mods" ? "Mods" : "Game log"}
          </button>
        ))}
      </div>

      {tab === "overview" ? (
        <Overview instance={instance} />
      ) : tab === "mods" && instance.loader !== "vanilla" ? (
        <ModsTab instance={instance} />
      ) : (
        <GameLog instance={instance} />
      )}
    </div>
  );
}

function Overview({ instance }: { instance: Instance }) {
  const settings = useApp((s) => s.settings);
  const system = useApp((s) => s.system);
  const upsert = useApp((s) => s.upsertInstance);
  const remove = useApp((s) => s.removeInstance);
  const key = instance.installKey ?? instance.gameVersion;
  const job = useVersionJob(key);
  const last = useApp((s) => s.lastFinished[key]);
  const installVersion = useApp((s) => s.installVersion);
  const running = useApp((s) => instance.id in s.running);
  const [error, setError] = useState<CmdError | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const useDefaultId = useId();

  const usesDefault = instance.maxMemoryMb === null;
  const mem = effectiveMemory(instance, settings);
  // Local value while dragging; persisted 350 ms after the last change.
  const [memDraft, setMemDraft] = useState(mem);
  const memTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => setMemDraft(mem), [mem]);
  useEffect(() => () => clearTimeout(memTimer.current), []);
  const onMemory = (v: number) => {
    setMemDraft(v);
    clearTimeout(memTimer.current);
    memTimer.current = setTimeout(() => void patch({ maxMemoryMb: v }), 350);
  };

  async function patch(p: { name?: string; maxMemoryMb?: number | null }) {
    setError(null);
    try {
      upsert(await api.updateInstance(instance.id, p));
      return true;
    } catch (e) {
      setError(toCmdError(e));
      return false;
    }
  }

  return (
    <div className="grid gap-4">
      {error && <InlineError error={error} />}

      <Card title="Memory">
        <div className="flex flex-col gap-5">
          <Toggle
            id={useDefaultId}
            checked={usesDefault}
            onChange={(v) => void patch({ maxMemoryMb: v ? null : mem })}
            label="Use launcher default"
            description={`Currently ${formatMemory(settings?.defaultMaxMemoryMb ?? 4096)}. Change it in Settings.`}
            disabled={running}
          />
          {!usesDefault && (
            <MemorySlider
              value={memDraft}
              onChange={onMemory}
              max={system?.maxMemoryMb ?? 16384}
              recommended={system?.recommendedMemoryMb}
              disabled={running}
            />
          )}
          {running && <p className="text-xs text-fg-faint">Changes apply the next time you play.</p>}
        </div>
      </Card>

      {instance.loader !== "vanilla" && <LoaderCard instance={instance} disabled={running || job.state !== "idle"} />}

      <InstanceJava instance={instance} disabled={running} />

      <JvmArgsCard instance={instance} disabled={running} />

      <Card title="Files">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <GameFolder instance={instance} disabled={running} onError={setError} />
          <div className="flex gap-2">
            <Button size="sm" icon={<FolderOpen size={13} />} onClick={() => void api.openInstanceDir(instance.id).catch((e) => setError(toCmdError(e)))}>
              Open folder
            </Button>
            <Button
              size="sm"
              icon={<RefreshCw size={13} />}
              disabled={job.state !== "idle" || running}
              onClick={() => void installVersion(key)}
              title="Re-check every game file against Mojang's hashes and re-download anything missing or damaged"
            >
              {instance.installed ? "Verify files" : "Download"}
            </Button>
          </div>
        </div>
        {last?.status === "completed" && last.repair && job.state === "idle" && (
          <p role="status" className={`mt-3 text-xs ${last.repairedCount > 0 ? "text-warning" : "text-success"}`}>
            {last.repairedCount > 0
              ? `Verified: ${last.repairedCount} damaged or missing ${last.repairedCount === 1 ? "file was" : "files were"} repaired. Details are in Downloads.`
              : "Verified: every file matches Mojang's hashes."}
          </p>
        )}
        <dl className="mt-4 grid grid-cols-3 gap-4 border-t border-line pt-4 text-xs">
          <div>
            <dt className="text-fg-faint">Created</dt>
            <dd className="mt-0.5 text-fg-muted">{new Date(instance.createdAt * 1000).toLocaleDateString()}</dd>
          </div>
          <div>
            <dt className="text-fg-faint">Last played</dt>
            <dd className="mt-0.5 text-fg-muted">{formatRelative(instance.lastPlayedAt)}</dd>
          </div>
          <div>
            <dt className="text-fg-faint">Play time</dt>
            <dd className="mt-0.5 text-fg-muted">{instance.playTimeSeconds > 0 ? formatDuration(instance.playTimeSeconds) : "None yet"}</dd>
          </div>
        </dl>
      </Card>

      <Card title="Manage">
        <div className="flex flex-wrap gap-2">
          <Button size="sm" icon={<Pencil size={13} />} onClick={() => setRenaming(true)}>
            Rename
          </Button>
          <Button size="sm" variant="danger" icon={<Trash2 size={13} />} disabled={running} onClick={() => setDeleting(true)}>
            Delete instance
          </Button>
        </div>
      </Card>

      <RenameDialog
        open={renaming}
        current={instance.name}
        onClose={() => setRenaming(false)}
        onSave={async (name) => {
          if (await patch({ name })) setRenaming(false);
        }}
      />
      <Dialog
        open={deleting}
        onClose={() => setDeleting(false)}
        title={`Delete “${instance.name}”?`}
        footer={
          <>
            <Button onClick={() => setDeleting(false)}>Cancel</Button>
            <Button
              variant="danger"
              onClick={async () => {
                try {
                  await api.deleteInstance(instance.id);
                  remove(instance.id);
                  navigate({ name: "instances" });
                } catch (e) {
                  setDeleting(false);
                  setError(toCmdError(e));
                }
              }}
            >
              Delete
            </Button>
          </>
        }
      >
        The instance is removed from Night Client. Its folder, including worlds and screenshots, is moved to the{" "}
        <code className="font-mono text-fg">trash</code> folder in your data directory so you can still recover it by hand.
      </Dialog>
    </div>
  );
}

function RenameDialog({ open, current, onClose, onSave }: { open: boolean; current: string; onClose: () => void; onSave: (name: string) => Promise<void> }) {
  const [value, setValue] = useState(current);
  const id = useId();
  useEffect(() => {
    if (open) setValue(current);
  }, [open, current]);
  const valid = value.trim().length > 0 && value.trim().length <= 64;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Rename instance"
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!valid} onClick={() => void onSave(value.trim())}>
            Save
          </Button>
        </>
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) void onSave(value.trim());
        }}
      >
        <Field label="Name" htmlFor={id}>
          <input id={id} className={inputClass} value={value} maxLength={64} onChange={(e) => setValue(e.target.value)} autoFocus />
        </Field>
      </form>
    </Dialog>
  );
}

function GameLog({ instance }: { instance: Instance }) {
  const lines = useApp((s) => s.logs[instance.id]);
  const clear = useApp((s) => s.clearLogs);
  const running = useApp((s) => instance.id in s.running);
  return (
    <section aria-label="Game log" className="flex min-h-[320px] flex-1 flex-col overflow-hidden rounded-lg border border-line bg-surface-1">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-line px-4">
        <span className="flex items-center gap-1.5 text-xs text-fg-muted">
          <span className={`size-1.5 rounded-full ${running ? "bg-success" : "bg-fg-faint"}`} aria-hidden="true" />
          {running ? "Running" : "Not running"}
        </span>
        <Button variant="ghost" size="sm" onClick={() => clear(instance.id)}>
          Clear
        </Button>
      </div>
      <div className="min-h-0 flex-1">
        <LogView lines={lines ?? []} label="Game output" empty="Output from this session appears here after you press Play. Minecraft's own logs are in the instance's logs folder." />
      </div>
    </section>
  );
}
