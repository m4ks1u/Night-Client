import { useEffect, useId, useState } from "react";
import { FolderInput, RotateCcw } from "lucide-react";
import { Button } from "./Button";
import { Card, Dialog, InlineError, inputClass } from "./ui";
import { LoaderVersionSelect } from "./LoaderPicker";
import { api, toCmdError, type CmdError, type Instance } from "../lib/ipc";
import { LOADER_NAME } from "../lib/keys";
import { useApp } from "../stores/app";

/** Loader version of a Fabric/NeoForge instance, with a change dialog. */
export function LoaderCard({ instance, disabled }: { instance: Instance; disabled: boolean }) {
  const upsert = useApp((s) => s.upsertInstance);
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState<string | null>(instance.loaderVersion);
  const [error, setError] = useState<CmdError | null>(null);
  const id = useId();
  const loader = instance.loader === "vanilla" ? null : instance.loader;

  useEffect(() => {
    if (open) {
      setValue(instance.loaderVersion);
      setError(null);
    }
  }, [open, instance.loaderVersion]);

  if (!loader) return null;
  return (
    <Card title="Mod loader">
      <div className="flex items-center justify-between gap-4">
        <div className="text-[13px]">
          <p className="font-medium">
            {LOADER_NAME[loader]} {instance.loaderVersion}
          </p>
          <p className="text-xs text-fg-faint">For Minecraft {instance.gameVersion}</p>
        </div>
        <Button size="sm" disabled={disabled} onClick={() => setOpen(true)}>
          Change version
        </Button>
      </div>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title={`${LOADER_NAME[loader]} version`}
        footer={
          <>
            <Button onClick={() => setOpen(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!value || value === instance.loaderVersion}
              onClick={async () => {
                try {
                  upsert(await api.setInstanceLoaderVersion(instance.id, value!));
                  setOpen(false);
                } catch (e) {
                  setError(toCmdError(e));
                }
              }}
            >
              Save
            </Button>
          </>
        }
      >
        <div className="mt-2 flex flex-col gap-2">
          <label htmlFor={id} className="text-[13px] font-medium text-fg">
            Version
          </label>
          {open && <LoaderVersionSelect id={id} loader={loader} minecraft={instance.gameVersion} value={value} onChange={setValue} />}
          <p className="text-xs">The new version is downloaded the next time you play. Mods may need updating too.</p>
          {error && <InlineError error={error} />}
        </div>
      </Dialog>
    </Card>
  );
}

/** Game folder: default (inside the data folder) or a custom location. */
export function GameFolder({ instance, disabled, onError }: { instance: Instance; disabled: boolean; onError: (e: CmdError | null) => void }) {
  const upsert = useApp((s) => s.upsertInstance);
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState("");
  const [error, setError] = useState<CmdError | null>(null);
  const id = useId();

  async function save(p: string | null) {
    setError(null);
    try {
      upsert(await api.setInstanceGameDir(instance.id, p));
      setOpen(false);
      onError(null);
    } catch (e) {
      if (open) setError(toCmdError(e));
      else onError(toCmdError(e));
    }
  }

  return (
    <div className="min-w-0 text-[13px] text-fg-muted">
      <p>
        Game folder{" "}
        <code className="font-mono text-fg" title={instance.gameDir ?? undefined}>
          {instance.gameDir ?? `instances\\${instance.dirName}`}
        </code>
        {instance.gameDir && <span className="ml-1 text-xs text-fg-faint">(custom)</span>}
      </p>
      <div className="mt-2 flex gap-2">
        <Button size="sm" variant="ghost" icon={<FolderInput size={13} />} disabled={disabled} onClick={() => { setPath(instance.gameDir ?? ""); setError(null); setOpen(true); }}>
          Change folder
        </Button>
        {instance.gameDir && (
          <Button size="sm" variant="ghost" icon={<RotateCcw size={13} />} disabled={disabled} onClick={() => void save(null)}>
            Use default
          </Button>
        )}
      </div>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title="Game folder"
        footer={
          <>
            <Button onClick={() => setOpen(false)}>Cancel</Button>
            <Button variant="primary" disabled={!path.trim()} onClick={() => void save(path.trim())}>
              Use this folder
            </Button>
          </>
        }
      >
        <p>Worlds, settings, screenshots and mods for this instance are stored here. Existing files are not moved.</p>
        <div className="mt-3 flex gap-2">
          <label htmlFor={id} className="sr-only">
            Folder path
          </label>
          <input id={id} className={`${inputClass} font-mono text-xs`} placeholder="D:\Minecraft\Survival" value={path} onChange={(e) => setPath(e.target.value)} />
          <Button
            onClick={async () => {
              const picked = await api.pickFolder().catch(() => null);
              if (picked) setPath(picked);
            }}
          >
            Browse…
          </Button>
        </div>
        {error && <div className="mt-3"><InlineError error={error} /></div>}
      </Dialog>
    </div>
  );
}
