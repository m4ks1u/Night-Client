import { useEffect, useId, useState } from "react";
import { ArrowLeft, Plus, RefreshCw, WifiOff } from "lucide-react";
import { Button } from "../components/Button";
import { Card, ErrorState, Field, InlineError, MemorySlider, PageHeader, Skeleton, Toggle, inputClass } from "../components/ui";
import { api, toCmdError, type CmdError, type LoaderKind } from "../lib/ipc";
import { LoaderSegments, LoaderVersionSelect } from "../components/LoaderPicker";
import { LOADER_NAME } from "../lib/keys";
import { defaultInstanceName } from "../lib/format";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";

export function NewInstance() {
  const [snapshots, setSnapshots] = useState(false);
  const versions = useApp((s) => s.versions[snapshots ? "all" : "releases"]);
  const loadVersions = useApp((s) => s.loadVersions);
  const settings = useApp((s) => s.settings);
  const system = useApp((s) => s.system);
  const upsert = useApp((s) => s.upsertInstance);
  const installVersion = useApp((s) => s.installVersion);
  const { navigate, back } = useRouter();

  const [version, setVersion] = useState("");
  const [loader, setLoader] = useState<LoaderKind>("vanilla");
  const [loaderVersion, setLoaderVersion] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [memory, setMemory] = useState<number | null>(null);
  const [downloadNow, setDownloadNow] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CmdError | null>(null);
  const ids = { name: useId(), version: useId(), snap: useId(), dl: useId(), lv: useId() };

  useEffect(() => void loadVersions(snapshots), [snapshots, loadVersions]);
  useEffect(() => {
    if (versions.status === "ready" && !versions.data.versions.some((v) => v.id === version)) {
      setVersion(versions.data.latestRelease);
    }
  }, [versions, version]);

  const effectiveName = nameTouched
    ? name
    : version
      ? loader === "vanilla"
        ? defaultInstanceName(version)
        : `${LOADER_NAME[loader]} ${version}`
      : "";
  const loaderReady = loader === "vanilla" || loaderVersion !== null;
  const defaultMem = settings?.defaultMaxMemoryMb ?? system?.recommendedMemoryMb ?? 4096;
  const nameError = nameTouched && (name.trim().length === 0 || name.trim().length > 64);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const inst = await api.createInstance({
        name: effectiveName.trim(),
        gameVersion: version,
        loader,
        loaderVersion: loader === "vanilla" ? null : loaderVersion,
        maxMemoryMb: memory === null || memory === defaultMem ? null : memory,
      });
      upsert(inst);
      navigate({ name: "instance", id: inst.id });
      if (downloadNow && !inst.installed && inst.installKey) void installVersion(inst.installKey);
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="max-w-2xl">
      <Button variant="ghost" size="sm" className="-ml-2 mb-3" icon={<ArrowLeft size={14} />} onClick={back}>
        Back
      </Button>
      <PageHeader title="New instance" subtitle="Minecraft: Java Edition, plain or with the Fabric or NeoForge mod loader." />

      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          if (!busy && version && !nameError && loaderReady) void create();
        }}
      >
        <Card>
          <div className="flex flex-col gap-5">
            <Field label="Name" htmlFor={ids.name} hint={nameError ? undefined : "You can rename it later. The folder name stays the same."}>
              <input
                id={ids.name}
                className={inputClass}
                value={effectiveName}
                maxLength={64}
                aria-invalid={nameError || undefined}
                onChange={(e) => {
                  setNameTouched(true);
                  setName(e.target.value);
                }}
              />
              {nameError && <p className="text-xs text-danger">Name must be 1–64 characters.</p>}
            </Field>

            <Field label="Minecraft version" htmlFor={ids.version}>
              {versions.status === "error" ? (
                <ErrorState title="Versions could not be loaded" error={versions.error} onRetry={() => void loadVersions(snapshots, true)} />
              ) : versions.status !== "ready" ? (
                <Skeleton className="h-9" />
              ) : (
                <>
                  <select id={ids.version} className={inputClass} value={version} onChange={(e) => setVersion(e.target.value)}>
                    {versions.data.versions.map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.id}
                        {v.id === versions.data.latestRelease ? " · latest release" : v.kind === "snapshot" ? " · snapshot" : ""}
                        {v.installed ? " · downloaded" : ""}
                      </option>
                    ))}
                  </select>
                  {versions.data.fromCache && (
                    <p className="flex items-center gap-1.5 text-xs text-warning">
                      <WifiOff size={12} /> Offline: showing the last saved version list.
                      <button type="button" className="underline" onClick={() => void loadVersions(snapshots, true)}>
                        <RefreshCw size={11} className="inline" /> Retry
                      </button>
                    </p>
                  )}
                </>
              )}
            </Field>

            <div className="flex flex-col gap-1.5">
              <span className="text-[13px] font-medium">Mod loader</span>
              <LoaderSegments value={loader} onChange={(l) => { setLoader(l); setLoaderVersion(null); }} />
              <p className="text-xs text-fg-faint">
                {loader === "vanilla"
                  ? "No mods. Exactly the game Mojang ships."
                  : loader === "fabric"
                    ? "Lightweight loader. Many performance and quality-of-life mods use it."
                    : "Successor to Forge, used by many large content mods."}
              </p>
            </div>

            {loader !== "vanilla" && version && (
              <Field label={`${LOADER_NAME[loader]} version`} htmlFor={ids.lv}>
                <LoaderVersionSelect id={ids.lv} loader={loader} minecraft={version} value={loaderVersion} onChange={setLoaderVersion} />
              </Field>
            )}

            <Toggle
              id={ids.snap}
              checked={snapshots}
              onChange={setSnapshots}
              label="Show snapshots"
              description="Development builds. They can corrupt worlds; back them up first."
            />
          </div>
        </Card>

        <Card>
          <div className="flex flex-col gap-5">
            <MemorySlider
              value={memory ?? defaultMem}
              onChange={setMemory}
              max={system?.maxMemoryMb ?? 16384}
              recommended={system?.recommendedMemoryMb}
            />
            <Toggle id={ids.dl} checked={downloadNow} onChange={setDownloadNow} label="Download now" description="Start downloading the game files right after creating the instance." />
          </div>
        </Card>

        {error && <InlineError error={error} />}

        <div className="flex justify-end gap-2">
          <Button onClick={back}>Cancel</Button>
          <Button type="submit" variant="primary" icon={<Plus size={15} />} disabled={busy || !version || nameError || !loaderReady}>
            Create instance
          </Button>
        </div>
      </form>
    </div>
  );
}
