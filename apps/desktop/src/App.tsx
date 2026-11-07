import { useCallback, useEffect, useId, useState } from "react";
import { AnimatePresence, LazyMotion, MotionConfig, domAnimation, m } from "motion/react";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FolderOpen,
  Loader2,
  LogIn,
  LogOut,
  Play,
  RefreshCw,
  Square,
  WifiOff,
  X,
} from "lucide-react";

import { TitleBar } from "./components/TitleBar";
import { Button } from "./components/Button";
import { ProgressBar } from "./components/ProgressBar";
import { LogView } from "./components/LogView";
import { api, events, toCmdError, type Account, type AppInfo, type CmdError, type VersionList } from "./lib/ipc";
import { PHASE_LABEL, formatBytes, formatMemory, fraction } from "./lib/format";
import { useLauncher } from "./stores/launcher";

const fade = {
  initial: { opacity: 0, y: 4 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -4 },
  transition: { duration: 0.16, ease: [0.2, 0.8, 0.2, 1] as const },
};

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [versions, setVersions] = useState<VersionList | null>(null);
  const [versionsError, setVersionsError] = useState<CmdError | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [account, setAccount] = useState<Account | null>(null);
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<CmdError | null>(null);
  const [memory, setMemory] = useState(4096);
  const [launchError, setLaunchError] = useState<CmdError | null>(null);

  const { install, game, startInstall, setProgress, finishInstall, failInstall, setGame, appendLogs, clearLogs } =
    useLauncher();

  const loadVersions = useCallback(async () => {
    setVersionsError(null);
    try {
      const v = await api.listVersions();
      setVersions(v);
      setSelected((cur) => cur || v.latestRelease);
    } catch (e) {
      setVersionsError(toCmdError(e));
    }
  }, []);

  useEffect(() => {
    void api.appInfo().then(setInfo);
    void api.account().then(setAccount);
    void loadVersions();
    const subs = [
      events.onInstallProgress(setProgress),
      events.onGameLog(appendLogs),
      events.onGameExit((e) => setGame({ status: "stopped", lastExit: e.code })),
    ];
    return () => subs.forEach((p) => void p.then((off) => off()));
  }, [loadVersions, setProgress, appendLogs, setGame]);

  const current = versions?.versions.find((v) => v.id === selected);
  const installing = install.status === "running";
  const installed = current?.installed ?? false;

  async function onInstall() {
    if (!selected) return;
    startInstall(selected);
    try {
      const r = await api.installVersion(selected);
      finishInstall(selected, r);
      setVersions((v) =>
        v && { ...v, versions: v.versions.map((x) => (x.id === selected ? { ...x, installed: true } : x)) },
      );
    } catch (e) {
      failInstall(selected, toCmdError(e));
    }
  }

  async function onSignIn() {
    setAuthBusy(true);
    setAuthError(null);
    try {
      setAccount(await api.signIn());
    } catch (e) {
      const err = toCmdError(e);
      if (err.code !== "cancelled") setAuthError(err);
    } finally {
      setAuthBusy(false);
    }
  }

  async function onLaunch() {
    setLaunchError(null);
    clearLogs();
    setGame({ status: "starting" });
    try {
      await api.launchGame(selected, memory);
      setGame({ status: "running" });
    } catch (e) {
      setGame({ status: "stopped" });
      setLaunchError(toCmdError(e));
    }
  }

  const progress = install.status === "running" ? install.progress : null;

  return (
    <MotionConfig reducedMotion="user">
      <LazyMotion features={domAnimation} strict>
        <div className="flex h-full flex-col">
          <TitleBar />
          <main className="grid min-h-0 flex-1 grid-cols-[minmax(340px,400px)_1fr] gap-px bg-line">
            {/* ------------------------------------------------ controls */}
            <section className="flex min-h-0 flex-col gap-5 overflow-y-auto bg-bg p-5" aria-label="Launcher controls">
              <header>
                <h1 className="text-[17px] font-semibold tracking-tight">Proof of concept</h1>
                <p className="mt-1 text-[13px] text-fg-muted">
                  Install an official release, sign in with Microsoft and launch it.
                </p>
              </header>

              {/* Account */}
              <Card title="Account">
                {account ? (
                  <div className="flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate font-medium">{account.name}</p>
                      <p className="text-xs text-fg-faint">Microsoft account · in memory only (Stage 0)</p>
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      icon={<LogOut size={14} />}
                      onClick={() => void api.signOut().then(() => setAccount(null))}
                    >
                      Sign out
                    </Button>
                  </div>
                ) : (
                  <div className="flex flex-col gap-2.5">
                    {authBusy ? (
                      <div className="flex items-center justify-between gap-3">
                        <p className="flex items-center gap-2 text-[13px] text-fg-muted">
                          <Loader2 size={14} className="animate-spin" /> Waiting for the browser…
                        </p>
                        <Button variant="ghost" size="sm" onClick={() => void api.cancelSignIn()}>
                          Cancel
                        </Button>
                      </div>
                    ) : (
                      <Button
                        variant="secondary"
                        icon={<LogIn size={15} />}
                        disabled={!info?.signInConfigured}
                        onClick={() => void onSignIn()}
                      >
                        Sign in with Microsoft
                      </Button>
                    )}
                    {info && !info.signInConfigured && (
                      <p className="text-xs leading-relaxed text-fg-faint">
                        Sign-in needs an approved Azure application ID. Set <code className="font-mono">NIGHT_CLIENT_MSA_CLIENT_ID</code> and restart.
                      </p>
                    )}
                    {authError && <InlineError error={authError} />}
                  </div>
                )}
              </Card>

              {/* Version */}
              <Card title="Minecraft version">
                {versionsError ? (
                  <div className="flex flex-col gap-2.5">
                    <InlineError error={versionsError} icon={<WifiOff size={14} />} />
                    <Button size="sm" icon={<RefreshCw size={13} />} onClick={() => void loadVersions()}>
                      Retry
                    </Button>
                  </div>
                ) : !versions ? (
                  <div className="h-9 animate-pulse rounded-md bg-surface-3" aria-label="Loading versions" />
                ) : (
                  <div className="flex flex-col gap-3">
                    <VersionSelect
                      versions={versions}
                      value={selected}
                      disabled={installing || game.status !== "stopped"}
                      onChange={setSelected}
                    />
                    {versions.fromCache && (
                      <p className="flex items-center gap-1.5 text-xs text-warning">
                        <WifiOff size={12} /> Offline — showing the cached version list.
                      </p>
                    )}
                    <div className="flex gap-2">
                      {installing ? (
                        <Button variant="danger" className="flex-1" icon={<X size={14} />} onClick={() => void api.cancelInstall()}>
                          Cancel
                        </Button>
                      ) : (
                        <Button
                          variant={installed ? "secondary" : "primary"}
                          className="flex-1"
                          icon={installed ? <RefreshCw size={14} /> : <Download size={15} />}
                          disabled={!selected || game.status !== "stopped"}
                          onClick={() => void onInstall()}
                        >
                          {installed ? "Verify & repair" : "Install"}
                        </Button>
                      )}
                    </div>

                    <AnimatePresence mode="wait" initial={false}>
                      {install.status === "running" && (
                        <m.div key="progress" {...fade} className="flex flex-col gap-2">
                          <div className="flex items-baseline justify-between text-xs">
                            <span className="text-fg-muted">{progress ? PHASE_LABEL[progress.phase] : "Preparing…"}</span>
                            {progress && progress.totalBytes > 0 && (
                              <span className="tabular-nums text-fg-faint">
                                {formatBytes(progress.doneBytes)} / {formatBytes(progress.totalBytes)}
                              </span>
                            )}
                          </div>
                          <ProgressBar value={progress && progress.totalFiles > 0 ? fraction(progress) : null} label="Installation progress" />
                          {progress && progress.totalFiles > 0 && (
                            <p className="text-2xs tabular-nums text-fg-faint">
                              {progress.doneFiles.toLocaleString()} of {progress.totalFiles.toLocaleString()} files
                            </p>
                          )}
                        </m.div>
                      )}
                      {install.status === "done" && install.versionId === selected && (
                        <m.p key="done" {...fade} className="flex items-center gap-1.5 text-xs text-success">
                          <CheckCircle2 size={13} />
                          {install.result.downloadedFiles === 0
                            ? `All ${install.result.skippedFiles.toLocaleString()} files verified.`
                            : `Installed — ${formatBytes(install.result.downloadedBytes)} downloaded.`}
                        </m.p>
                      )}
                      {install.status === "error" && install.versionId === selected && (
                        <m.div key="err" {...fade}>
                          <InlineError error={install.error} />
                        </m.div>
                      )}
                    </AnimatePresence>
                  </div>
                )}
              </Card>

              {/* Play */}
              <Card title="Play">
                <MemorySlider value={memory} onChange={setMemory} disabled={game.status !== "stopped"} />
                <div className="mt-4">
                  {game.status === "stopped" ? (
                    <Button
                      variant="primary"
                      size="lg"
                      className="w-full"
                      icon={<Play size={16} fill="currentColor" />}
                      disabled={!installed || installing || !account}
                      onClick={() => void onLaunch()}
                    >
                      Launch {selected}
                    </Button>
                  ) : (
                    <Button
                      variant="danger"
                      size="lg"
                      className="w-full"
                      icon={game.status === "starting" ? <Loader2 size={16} className="animate-spin" /> : <Square size={14} fill="currentColor" />}
                      disabled={game.status === "starting"}
                      onClick={() => void api.stopGame()}
                    >
                      {game.status === "starting" ? "Starting…" : "Force stop"}
                    </Button>
                  )}
                </div>
                {game.status === "stopped" && (!installed || !account) && (
                  <p className="mt-2 text-xs text-fg-faint">
                    {!installed ? "Install this version first." : "Sign in with a Microsoft account that owns Minecraft: Java Edition."}
                  </p>
                )}
                {launchError && <div className="mt-2"><InlineError error={launchError} /></div>}
                {game.status === "stopped" && game.lastExit !== undefined && game.lastExit !== 0 && (
                  <p className="mt-2 text-xs text-warning">Game exited with code {String(game.lastExit)}. See the log for details.</p>
                )}
              </Card>

              <footer className="mt-auto flex items-center justify-between pt-2 text-xs text-fg-faint">
                <span>v{info?.version ?? "…"} · Not affiliated with Mojang or Microsoft</span>
                <Button variant="ghost" size="sm" icon={<FolderOpen size={13} />} onClick={() => void api.openDataDir()}>
                  Data folder
                </Button>
              </footer>
            </section>

            {/* ------------------------------------------------ log */}
            <section className="flex min-h-0 flex-col bg-surface-1" aria-label="Game log">
              <div className="flex h-11 shrink-0 items-center justify-between border-b border-line px-4">
                <h2 className="text-[13px] font-medium">Game output</h2>
                <div className="flex items-center gap-3">
                  <StatusDot status={game.status} />
                  <Button variant="ghost" size="sm" onClick={clearLogs}>
                    Clear
                  </Button>
                </div>
              </div>
              <div className="min-h-0 flex-1">
                <LogView />
              </div>
            </section>
          </main>
        </div>
      </LazyMotion>
    </MotionConfig>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="rounded-lg border border-line bg-surface-1 p-4">
      <h2 id={id} className="mb-3 text-2xs font-semibold tracking-[0.08em] text-fg-faint uppercase">
        {title}
      </h2>
      {children}
    </section>
  );
}

function InlineError({ error, icon }: { error: CmdError; icon?: React.ReactNode }) {
  return (
    <p role="alert" className="flex items-start gap-2 rounded-md bg-danger-soft px-3 py-2 text-xs leading-relaxed text-danger">
      <span className="mt-0.5 shrink-0">{icon ?? <AlertTriangle size={13} />}</span>
      <span className="select-text">{error.message}</span>
    </p>
  );
}

function VersionSelect(props: { versions: VersionList; value: string; disabled: boolean; onChange: (v: string) => void }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="sr-only">
        Version
      </label>
      <select
        id={id}
        value={props.value}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.value)}
        className="h-9 w-full rounded-md border border-line bg-surface-2 px-3 text-sm text-fg transition-colors hover:border-line-strong disabled:opacity-50"
      >
        {props.versions.versions.map((v) => (
          <option key={v.id} value={v.id}>
            {v.id}
            {v.id === props.versions.latestRelease ? "  · latest" : ""}
            {v.installed ? "  · installed" : ""}
          </option>
        ))}
      </select>
    </div>
  );
}

function MemorySlider({ value, onChange, disabled }: { value: number; onChange: (v: number) => void; disabled: boolean }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-[13px] text-fg-muted">
          Memory
        </label>
        <span className="text-[13px] font-medium tabular-nums">{formatMemory(value)}</span>
      </div>
      <input
        id={id}
        type="range"
        min={1024}
        max={16384}
        step={512}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full accent-accent disabled:opacity-50"
      />
    </div>
  );
}

function StatusDot({ status }: { status: "stopped" | "starting" | "running" }) {
  const map = {
    stopped: ["bg-fg-faint", "Not running"],
    starting: ["bg-warning", "Starting"],
    running: ["bg-success", "Running"],
  } as const;
  const [color, label] = map[status];
  return (
    <span className="flex items-center gap-1.5 text-xs text-fg-muted">
      <span className={`size-1.5 rounded-full ${color}`} aria-hidden="true" />
      {label}
    </span>
  );
}
