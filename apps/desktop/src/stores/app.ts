import { create } from "zustand";
import {
  api,
  toCmdError,
  type Account,
  type AppInfo,
  type CmdError,
  type Instance,
  type InstallProgress,
  type LogLine,
  type Settings,
  type SystemInfo,
  type VersionList,
} from "../lib/ipc";

export const MAX_LOG_LINES = 2000;

type Load<T> = { status: "idle" | "loading" } | { status: "ready"; data: T } | { status: "error"; error: CmdError };

type InstallState = {
  versionId: string;
  progress: InstallProgress | null;
};

type AppStore = {
  info: AppInfo | null;
  system: SystemInfo | null;
  settings: Settings | null;
  account: Account | null;
  instances: Load<Instance[]>;
  versions: Record<"releases" | "all", Load<VersionList>>;
  /** Installation in progress (only one at a time). */
  install: InstallState | null;
  installErrors: Record<string, CmdError>;
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

  installVersion: (versionId: string) => Promise<boolean>;
  setProgress: (p: InstallProgress) => void;

  markStarted: (instanceId: string) => void;
  markExited: (instanceId: string) => void;
  appendLogs: (instanceId: string, lines: LogLine[]) => void;
  clearLogs: (instanceId: string) => void;
};

export const useApp = create<AppStore>((set, get) => ({
  info: null,
  system: null,
  settings: null,
  account: null,
  instances: { status: "idle" },
  versions: { releases: { status: "idle" }, all: { status: "idle" } },
  install: null,
  installErrors: {},
  running: {},
  logs: {},

  boot: async () => {
    const [info, system, account, running, current] = await Promise.all([
      api.appInfo(),
      api.systemInfo(),
      api.account(),
      api.runningInstances(),
      api.currentInstall(),
    ]);
    set({
      info,
      system,
      account,
      running: Object.fromEntries(running.map((r) => [r.instanceId, r.startedAt])),
      install: current ? { versionId: current, progress: null } : null,
    });
    if (!info.startupError) {
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
      s.instances.status === "ready"
        ? { instances: { status: "ready", data: s.instances.data.filter((x) => x.id !== id) } }
        : s,
    ),

  setAccount: (account) => set({ account }),
  setSettings: (settings) => set({ settings }),

  installVersion: async (versionId) => {
    set((s) => {
      const { [versionId]: _, ...rest } = s.installErrors;
      return { install: { versionId, progress: null }, installErrors: rest };
    });
    try {
      await api.installVersion(versionId);
      set({ install: null });
      const markInstalled = (l: Load<VersionList>): Load<VersionList> =>
        l.status === "ready"
          ? { status: "ready", data: { ...l.data, versions: l.data.versions.map((v) => (v.id === versionId ? { ...v, installed: true } : v)) } }
          : l;
      set((s) => ({ versions: { releases: markInstalled(s.versions.releases), all: markInstalled(s.versions.all) } }));
      void get().refreshInstances();
      return true;
    } catch (e) {
      const err = toCmdError(e);
      set((s) => ({
        install: null,
        installErrors: err.code === "cancelled" ? s.installErrors : { ...s.installErrors, [versionId]: err },
      }));
      return false;
    }
  },

  setProgress: (p) =>
    set((s) => (s.install && s.install.versionId === p.id ? { install: { ...s.install, progress: p } } : s)),

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

/** Effective memory for an instance (its override or the launcher default). */
export function effectiveMemory(i: Instance, s: Settings | null): number {
  return i.maxMemoryMb ?? s?.defaultMaxMemoryMb ?? 4096;
}
