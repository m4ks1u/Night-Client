import { create } from "zustand";
import {
  api,
  toCmdError,
  type Account,
  type AppInfo,
  type CmdError,
  type InstallFinished,
  type Instance,
  type InstallProgress,
  type LogLine,
  type QueueSnapshot,
  type Settings,
  type SystemInfo,
  type VersionList,
} from "../lib/ipc";

export const MAX_LOG_LINES = 2000;

type Load<T> = { status: "idle" | "loading" } | { status: "ready"; data: T } | { status: "error"; error: CmdError };

type AppStore = {
  info: AppInfo | null;
  system: SystemInfo | null;
  settings: Settings | null;
  account: Account | null;
  instances: Load<Instance[]>;
  versions: Record<"releases" | "all", Load<VersionList>>;
  /** Download queue mirrored from the backend. */
  queue: QueueSnapshot;
  /** Progress of the active download job. */
  progress: InstallProgress | null;
  /** Last failure per version (cleared when it is queued again). */
  installErrors: Record<string, CmdError>;
  /** Last completed job per version (for "verified, N repaired" messages). */
  lastFinished: Record<string, InstallFinished>;
  /** Bumped whenever a job finishes so history views can refetch. */
  historyVersion: number;
  /** instanceId -> unix start time */
  running: Record<string, number>;
  logs: Record<string, LogLine[]>;

  boot: () => Promise<void>;
  refreshInstances: () => Promise<void>;
  loadVersions: (includeSnapshots: boolean, force?: boolean) => Promise<void>;
  upsertInstance: (i: Instance) => void;
  removeInstance: (id: string) => void;
  setAccount: (a: Account | null) => void;
  setSettings: (s: Settings) => void;

  /** Queue an install/repair and resolve when it finishes (true = completed). */
  installVersion: (versionId: string) => Promise<boolean>;
  setQueue: (q: QueueSnapshot) => void;
  setProgress: (p: InstallProgress) => void;
  onFinished: (f: InstallFinished) => void;

  markStarted: (instanceId: string) => void;
  markExited: (instanceId: string) => void;
  appendLogs: (instanceId: string, lines: LogLine[]) => void;
  clearLogs: (instanceId: string) => void;
};

/** Promises waiting for a version's job to finish. */
const waiters = new Map<string, ((ok: boolean) => void)[]>();

export const useApp = create<AppStore>((set, get) => ({
  info: null,
  system: null,
  settings: null,
  account: null,
  instances: { status: "idle" },
  versions: { releases: { status: "idle" }, all: { status: "idle" } },
  queue: { active: null, pending: [] },
  progress: null,
  installErrors: {},
  lastFinished: {},
  historyVersion: 0,
  running: {},
  logs: {},

  boot: async () => {
    const [info, system, account, running, queue] = await Promise.all([
      api.appInfo(),
      api.systemInfo(),
      api.account(),
      api.runningInstances(),
      api.downloadQueue(),
    ]);
    set({ info, system, account, queue, running: Object.fromEntries(running.map((r) => [r.instanceId, r.startedAt])) });
    if (!info.startupError) {
      // Renew the saved sign-in in the background so problems show before Play.
      api.checkActiveAccount().then((account) => set({ account })).catch(() => {});
      api.getSettings().then((settings) => set({ settings })).catch(() => {});
      void get().refreshInstances();
    }
  },

  refreshInstances: async () => {
    if (get().instances.status !== "ready") set({ instances: { status: "loading" } });
    try {
      set({ instances: { status: "ready", data: await api.listInstances() } });
    } catch (e) {
      set({ instances: { status: "error", error: toCmdError(e) } });
    }
  },

  loadVersions: async (includeSnapshots, force = false) => {
    const key = includeSnapshots ? "all" : "releases";
    const cur = get().versions[key];
    if (!force && (cur.status === "ready" || cur.status === "loading")) return;
    set((s) => ({ versions: { ...s.versions, [key]: { status: "loading" } } }));
    try {
      const data = await api.listVersions(includeSnapshots);
      set((s) => ({ versions: { ...s.versions, [key]: { status: "ready", data } } }));
    } catch (e) {
      set((s) => ({ versions: { ...s.versions, [key]: { status: "error", error: toCmdError(e) } } }));
    }
  },

  upsertInstance: (i) =>
    set((s) => {
      if (s.instances.status !== "ready") return s;
      const exists = s.instances.data.some((x) => x.id === i.id);
      const data = exists ? s.instances.data.map((x) => (x.id === i.id ? i : x)) : [i, ...s.instances.data];
      return { instances: { status: "ready", data } };
    }),

  removeInstance: (id) =>
    set((s) =>
      s.instances.status === "ready" ? { instances: { status: "ready", data: s.instances.data.filter((x) => x.id !== id) } } : s,
    ),

  setAccount: (account) => set({ account }),
  setSettings: (settings) => set({ settings }),

  installVersion: async (versionId) => {
    set((s) => {
      const { [versionId]: _, ...rest } = s.installErrors;
      return { installErrors: rest };
    });
    const done = new Promise<boolean>((resolve) => waiters.set(versionId, [...(waiters.get(versionId) ?? []), resolve]));
    try {
      await api.installVersion(versionId);
    } catch (e) {
      waiters.delete(versionId);
      set((s) => ({ installErrors: { ...s.installErrors, [versionId]: toCmdError(e) } }));
      return false;
    }
    return done;
  },

  setQueue: (queue) => set((s) => ({ queue, progress: queue.active && s.progress?.jobId === queue.active.id ? s.progress : null })),

  setProgress: (p) => set((s) => (s.queue.active?.id === p.jobId ? { progress: p } : s)),

  onFinished: (f) => {
    for (const resolve of waiters.get(f.id) ?? []) resolve(f.status === "completed");
    waiters.delete(f.id);
    set((s) => {
      const installErrors = { ...s.installErrors };
      if (f.status === "failed") installErrors[f.id] = { code: f.code ?? "unknown", message: f.error ?? "Download failed." };
      const markInstalled = (l: Load<VersionList>): Load<VersionList> =>
        l.status === "ready" && f.status === "completed"
          ? { status: "ready", data: { ...l.data, versions: l.data.versions.map((v) => (v.id === f.id ? { ...v, installed: true } : v)) } }
          : l;
      return {
        installErrors,
        lastFinished: { ...s.lastFinished, [f.id]: f },
        historyVersion: s.historyVersion + 1,
        versions: { releases: markInstalled(s.versions.releases), all: markInstalled(s.versions.all) },
      };
    });
    void get().refreshInstances();
  },

  markStarted: (id) => set((s) => ({ running: { ...s.running, [id]: Date.now() / 1000 } })),
  markExited: (id) =>
    set((s) => {
      const { [id]: _, ...rest } = s.running;
      return { running: rest };
    }),

  appendLogs: (id, lines) =>
    set((s) => {
      const prev = s.logs[id] ?? [];
      const merged = prev.length + lines.length > MAX_LOG_LINES ? [...prev, ...lines].slice(-MAX_LOG_LINES) : [...prev, ...lines];
      return { logs: { ...s.logs, [id]: merged } };
    }),
  clearLogs: (id) => set((s) => ({ logs: { ...s.logs, [id]: [] } })),
}));

/** Download state of one version: active (with progress), queued, or idle. */
export function useVersionJob(versionId: string) {
  // Two primitive selectors: zustand v5 hooks compare with Object.is, so
  // returning a fresh object would re-render forever.
  const jobId = useApp((s) =>
    s.queue.active?.target === versionId ? s.queue.active.id : (s.queue.pending.find((j) => j.target === versionId)?.id ?? null),
  );
  const active = useApp((s) => s.queue.active?.target === versionId);
  const state: "active" | "queued" | "idle" = active ? "active" : jobId !== null ? "queued" : "idle";
  return { state, jobId };
}

/** Effective memory for an instance (its override or the launcher default). */
export function effectiveMemory(i: Instance, s: Settings | null): number {
  return i.maxMemoryMb ?? s?.defaultMaxMemoryMb ?? 4096;
}
