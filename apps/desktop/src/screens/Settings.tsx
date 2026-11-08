import { useEffect, useId, useRef, useState } from "react";
import { Check, FolderOpen } from "lucide-react";
import { Button } from "../components/Button";
import { Card, Field, InlineError, MemorySlider, PageHeader, Skeleton, Toggle, inputClass } from "../components/ui";
import { api, toCmdError, type CmdError, type Settings as SettingsT } from "../lib/ipc";
import { useApp } from "../stores/app";
import { JavaList } from "../components/JavaPicker";

export function Settings() {
  const settings = useApp((s) => s.settings);
  const setSettings = useApp((s) => s.setSettings);
  const system = useApp((s) => s.system);
  const info = useApp((s) => s.info);
  const [draft, setDraft] = useState<SettingsT | null>(settings);
  const [error, setError] = useState<CmdError | null>(null);
  const [saved, setSaved] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const ids = { conc: useId(), hide: useId() };

  useEffect(() => {
    if (settings && !draft) setDraft(settings);
  }, [settings, draft]);

  // Save shortly after the last change so dragging a slider is one write.
  function update(next: SettingsT) {
    setDraft(next);
    setSaved(false);
    clearTimeout(timer.current);
    timer.current = setTimeout(async () => {
      try {
        setSettings(await api.updateSettings(next));
        setError(null);
        setSaved(true);
      } catch (e) {
        setError(toCmdError(e));
      }
    }, 350);
  }
  useEffect(() => () => clearTimeout(timer.current), []);

  return (
    <div className="max-w-2xl">
      <PageHeader
        title="Settings"
        subtitle="Changes are saved automatically."
        actions={saved && (
          <span className="flex items-center gap-1 text-xs text-success" role="status">
            <Check size={13} /> Saved
          </span>
        )}
      />
      {error && <div className="mb-4"><InlineError error={error} /></div>}

      {!draft ? (
        <Skeleton className="h-64" />
      ) : (
        <div className="flex flex-col gap-4">
          <Card title="Game">
            <div className="flex flex-col gap-6">
              <MemorySlider
                label="Default memory for instances"
                value={draft.defaultMaxMemoryMb}
                onChange={(v) => update({ ...draft, defaultMaxMemoryMb: v })}
                max={system?.maxMemoryMb ?? 16384}
                recommended={system?.recommendedMemoryMb}
              />
              <Toggle
                id={ids.hide}
                checked={draft.hideOnLaunch}
                onChange={(v) => update({ ...draft, hideOnLaunch: v })}
                label="Hide Night Client while playing"
                description="The window comes back when the game closes."
              />
            </div>
          </Card>

          <JavaList />

          <Card title="Downloads">
            <Field label="Parallel downloads" htmlFor={ids.conc} hint="Lower this on slow or shared connections. Applies to the next download.">
              <select
                id={ids.conc}
                className={`${inputClass} w-32`}
                value={draft.downloadConcurrency}
                onChange={(e) => update({ ...draft, downloadConcurrency: Number(e.target.value) })}
              >
                {[1, 2, 4, 6, 8, 12, 16].map((n) => (
                  <option key={n} value={n}>
                    {n}
                    {n === 8 ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </Field>
          </Card>

          <Card title="Storage">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <p className="text-[13px] font-medium">Data folder</p>
                <p className="truncate font-mono text-xs text-fg-faint" title={info?.dataDir}>{info?.dataDir}</p>
              </div>
              <Button size="sm" icon={<FolderOpen size={13} />} onClick={() => void api.openDataDir()}>
                Open
              </Button>
            </div>
          </Card>

          <Card title="About">
            <div className="text-[13px] leading-relaxed text-fg-muted">
              <p>
                Night Client {info?.version} · Apache-2.0 · Geist fonts under SIL OFL 1.1
              </p>
              <p className="mt-2 text-xs text-fg-faint">
                Not an official Minecraft product. Not approved by or associated with Mojang Studios or Microsoft. Game files are
                downloaded from official servers and verified; none are distributed with Night Client.
              </p>
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
