import { useEffect, useId, useState } from "react";
import { Coffee, RefreshCw } from "lucide-react";
import { Button } from "./Button";
import { Badge, Card, Dialog, InlineError, Skeleton, inputClass } from "./ui";
import { api, toCmdError, type CmdError, type Compatibility, type Instance, type JavaRuntime } from "../lib/ipc";
import { useApp } from "../stores/app";

const COMPAT: Record<Compatibility, { label: string; tone: "success" | "accent" | "warning" | "neutral" }> = {
  recommended: { label: "Recommended", tone: "success" },
  compatible: { label: "Compatible", tone: "accent" },
  risky: { label: "May not work", tone: "warning" },
  incompatible: { label: "Incompatible", tone: "neutral" },
};

const SOURCE: Record<JavaRuntime["source"], string> = {
  managed: "Night Client (Mojang)",
  javaHome: "JAVA_HOME",
  path: "PATH",
  programFiles: "Installed",
  minecraftLauncher: "Minecraft Launcher",
  custom: "Custom",
};

export function CompatBadge({ c }: { c: Compatibility | null }) {
  if (!c) return null;
  return <Badge tone={COMPAT[c].tone}>{COMPAT[c].label}</Badge>;
}

function RuntimeLine({ r }: { r: JavaRuntime }) {
  return (
    <span className="min-w-0">
      <span className="flex flex-wrap items-center gap-2 text-[13px] font-medium">
        Java {r.version}
        <span className="text-xs font-normal text-fg-faint">{r.vendor ?? SOURCE[r.source]}</span>
        <CompatBadge c={r.compatibility} />
        {r.arch !== "x64" && <Badge tone="warning">{r.arch}</Badge>}
      </span>
      <span className="block truncate font-mono text-[11px] text-fg-faint" title={r.path}>
        {r.path}
      </span>
    </span>
  );
}

/** Instance-level Java choice: automatic (official runtime) or a specific runtime. */
export function InstanceJava({ instance, disabled }: { instance: Instance; disabled: boolean }) {
  const refresh = useApp((s) => s.refreshInstances);
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<JavaRuntime[] | null>(null);
  const [selected, setSelected] = useState<string | null>(instance.javaPath);
  const [custom, setCustom] = useState("");
  const [error, setError] = useState<CmdError | null>(null);
  const [saving, setSaving] = useState(false);
  const customId = useId();
  const group = useId();

  async function scan() {
    setList(null);
    try {
      setList(await api.detectJava(instance.gameVersion));
    } catch (e) {
      setError(toCmdError(e));
      setList([]);
    }
  }

  useEffect(() => {
    if (open) {
      setSelected(instance.javaPath);
      setError(null);
      void scan();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  async function save(path: string | null) {
    setSaving(true);
    setError(null);
    try {
      await api.setInstanceJava(instance.id, path);
      await refresh();
      setOpen(false);
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card title="Java">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 text-[13px]">
          {instance.javaPath ? (
            <>
              <p className="font-medium">Custom runtime</p>
              <p className="truncate font-mono text-[11px] text-fg-faint" title={instance.javaPath}>
                {instance.javaPath}
              </p>
            </>
          ) : (
            <>
              <p className="font-medium">Automatic</p>
              <p className="text-xs text-fg-faint">The official Mojang runtime for this version. Recommended.</p>
            </>
          )}
        </div>
        <Button size="sm" disabled={disabled} onClick={() => setOpen(true)}>
          Change
        </Button>
      </div>

      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title="Choose Java"
        footer={
          <>
            <Button onClick={() => setOpen(false)}>Cancel</Button>
            <Button variant="primary" disabled={saving || (selected === "__custom" && !custom.trim())} onClick={() => void save(selected === "__custom" ? custom.trim() : selected)}>
              Save
            </Button>
          </>
        }
      >
        <div role="radiogroup" aria-label="Java runtime" className="mt-2 flex max-h-[50vh] flex-col gap-1.5 overflow-y-auto">
          <Option name={group} checked={selected === null} onSelect={() => setSelected(null)}>
            <span className="text-[13px] font-medium">Automatic (recommended)</span>
            <span className="block text-xs text-fg-faint">Night Client installs the exact runtime Mojang specifies.</span>
          </Option>
          {list === null ? (
            <Skeleton className="h-12" />
          ) : (
            list.map((r) => (
                <Option key={r.path} name={group} checked={selected === r.path} disabled={r.compatibility === "incompatible" || r.arch !== "x64"} onSelect={() => setSelected(r.path)}>
                  <RuntimeLine r={r} />
                </Option>
            ))
          )}
          <Option name={group} checked={selected === "__custom"} onSelect={() => setSelected("__custom")}>
            <label htmlFor={customId} className="text-[13px] font-medium">
              Other location
            </label>
            <input
              id={customId}
              className={`${inputClass} mt-1.5 font-mono text-xs`}
              placeholder="C:\Program Files\Java\jdk-21\bin\javaw.exe"
              value={custom}
              onFocus={() => setSelected("__custom")}
              onChange={(e) => setCustom(e.target.value)}
            />
          </Option>
        </div>
        {list && list.length === 0 && (
          <p className="mt-2 text-xs text-fg-faint">No Java installations were found yet.</p>
        )}
        {error && <div className="mt-3"><InlineError error={error} /></div>}
      </Dialog>
    </Card>
  );
}

function Option({ name, checked, disabled, onSelect, children }: { name: string; checked: boolean; disabled?: boolean; onSelect: () => void; children: React.ReactNode }) {
  return (
    <label
      className={`flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2.5 transition-colors ${
        checked ? "border-accent/60 bg-accent-soft" : "border-line hover:border-line-strong"
      } ${disabled ? "cursor-not-allowed opacity-50" : ""}`}
    >
      <input type="radio" name={name} checked={checked} disabled={disabled} onChange={onSelect} className="mt-1 accent-accent" />
      <span className="min-w-0 flex-1">{children}</span>
    </label>
  );
}

/** Settings: everything found on this PC. */
export function JavaList() {
  const [list, setList] = useState<JavaRuntime[] | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  async function scan() {
    setList(null);
    setError(null);
    try {
      setList(await api.detectJava(null));
    } catch (e) {
      setError(toCmdError(e));
    }
  }
  useEffect(() => void scan(), []);
  return (
    <Card
      title="Java runtimes"
      actions={
        <Button size="sm" variant="ghost" icon={<RefreshCw size={13} />} onClick={() => void scan()}>
          Rescan
        </Button>
      }
    >
      <p className="mb-3 text-xs text-fg-faint">
        Instances use the official Mojang runtime automatically. You can choose one of these per instance in its Java settings.
      </p>
      {error ? (
        <InlineError error={error} />
      ) : !list ? (
        <Skeleton className="h-20" />
      ) : list.length === 0 ? (
        <p className="flex items-center gap-2 text-[13px] text-fg-muted">
          <Coffee size={14} /> No Java found yet. One is installed with your first game.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {list.map((r) => (
            <li key={r.path} className="flex items-center gap-3 rounded-md border border-line px-3 py-2">
              <RuntimeLine r={r} />
              <span className="ml-auto shrink-0 text-2xs text-fg-faint">{SOURCE[r.source]}</span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
