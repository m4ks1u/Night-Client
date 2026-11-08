// Typed bridge to the Rust commands in src-tauri/src/commands/.
// This file is the only place that names IPC commands and events.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type CmdError = { code: string; message: string };

export function toCmdError(e: unknown): CmdError {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) {
    return { code: String(e.code), message: String(e.message) };
  }
  return { code: "unknown", message: e instanceof Error ? e.message : String(e) };
}

export type AppInfo = {
  version: string;
  dataDir: string;
  logDir: string;
  signInConfigured: boolean;
  startupError: string | null;
};
export type SystemInfo = {
  totalMemoryMb: number;
  availableMemoryMb: number;
  osVersion: string;
  cpuThreads: number;
  recommendedMemoryMb: number;
  maxMemoryMb: number;
};
export type Settings = { defaultMaxMemoryMb: number; downloadConcurrency: number; hideOnLaunch: boolean; reduceMotion: boolean };
export type VersionItem = { id: string; kind: "release" | "snapshot" | string; releaseTime: string; installed: boolean };
export type VersionList = { latestRelease: string; latestSnapshot: string; versions: VersionItem[]; fromCache: boolean };
export type AccountStatus = "ready" | "signedIn" | "expired";
export type Account = { uuid: string; name: string; active: boolean; lastUsedAt: number | null; status: AccountStatus };

export type Instance = {
  id: string;
  name: string;
  dirName: string;
  gameVersion: string;
  loader: LoaderKind;
  loaderVersion: string | null;
  maxMemoryMb: number | null;
  javaPath: string | null;
  jvmArgs: string[];
  gameDir: string | null;
  /** Download-queue key, e.g. "fabric:1.21.1:0.17.2". */
  installKey: string | null;
  /** e.g. "Fabric 0.17.2 · 1.21.1" */
  label: string;
  createdAt: number;
  lastPlayedAt: number | null;
  playTimeSeconds: number;
  installed: boolean;
  running: boolean;
};
export type LoaderKind = "vanilla" | "fabric" | "neoforge";
export type LoaderVersion = { version: string; stable: boolean };
export type SortIndex = "relevance" | "downloads" | "follows" | "newest" | "updated";
export type SearchHit = {
  project_id: string;
  slug: string | null;
  title: string;
  description: string;
  icon_url: string | null;
  downloads: number;
  follows: number;
  author: string;
  display_categories: string[];
  installed: boolean;
};
export type SearchView = { hits: SearchHit[]; total: number; offset: number };
export type Category = { name: string; project_type: string; header: string };
export type MrVersion = {
  id: string;
  project_id: string;
  name: string;
  version_number: string;
  version_type: string;
  files: { filename: string; size: number; primary: boolean }[];
};
export type InstallPlan = {
  install: { version: MrVersion; requiredBy: string | null }[];
  alreadyInstalled: string[];
  missing: string[];
  conflicts: string[];
  titles: Record<string, string>;
};
export type ModRecord = {
  instanceId: string;
  fileName: string;
  source: "modrinth" | "local";
  projectId: string | null;
  versionId: string | null;
  title: string;
  versionNumber: string | null;
  iconUrl: string | null;
  size: number;
  enabled: boolean;
  requiredBy: string | null;
  installedAt: number;
};
export type ModUpdate = { fileName: string; current: string | null; versionId: string; versionNumber: string };
export type Running = { instanceId: string; startedAt: number };

export type Diagnostics = {
  appVersion: string;
  os: string;
  totalMemoryMb: number;
  cpuThreads: number;
  dataDir: string;
  logDir: string;
  logFile: string | null;
  database: string | null;
  schemaVersion: number | null;
  startupError: string | null;
  instanceCount: number | null;
  installedVersions: string[];
  runtimes: { component: string; version: string }[];
  signInConfigured: boolean;
};

export type InstallPhase = "metadata" | "runtime" | "gameFiles" | "loader";
export type InstallProgress = {
  jobId: number;
  id: string;
  phase: InstallPhase;
  totalFiles: number;
  doneFiles: number;
  skippedFiles: number;
  totalBytes: number;
  doneBytes: number;
};
export type DownloadStatus = "completed" | "failed" | "cancelled";
export type InstallFinished = {
  jobId: number;
  id: string;
  status: DownloadStatus;
  code: string | null;
  error: string | null;
  repair: boolean;
  repairedCount: number;
};
export type JobView = { id: number; kind: "version"; target: string; queuedAt: number };
export type QueueSnapshot = { active: JobView | null; pending: JobView[] };
export type DownloadRecord = {
  id: number;
  kind: string;
  target: string;
  status: DownloadStatus;
  repair: boolean;
  queuedAt: number;
  startedAt: number;
  finishedAt: number;
  filesDownloaded: number;
  filesVerified: number;
  bytesDownloaded: number;
  repaired: string[];
  error: string | null;
};
export type Compatibility = "recommended" | "compatible" | "risky" | "incompatible";
export type JavaRuntime = {
  path: string;
  home: string;
  version: string;
  major: number;
  vendor: string | null;
  arch: "x64" | "x86" | "arm64" | "unknown";
  source: "managed" | "javaHome" | "path" | "programFiles" | "minecraftLauncher" | "custom";
  compatibility: Compatibility | null;
};
export type LogLine = { stream: "stdout" | "stderr"; level: string | null; text: string };
export type LogBatch = { instanceId: string; lines: LogLine[] };
export type GameExit = { instanceId: string; code: number | null; seconds: number };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  systemInfo: () => invoke<SystemInfo>("system_info"),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  diagnostics: () => invoke<Diagnostics>("diagnostics"),
  recentLogs: (lines: number) => invoke<string[]>("recent_logs", { lines }),
  openDataDir: () => invoke<void>("open_data_dir"),
  /** Opens the Windows 11 snap-layout flyout (no-op unless focused and not maximized). */
  showSnapLayouts: () => invoke<void>("show_snap_layouts"),
  openLogsDir: () => invoke<void>("open_logs_dir"),

  listVersions: (includeSnapshots: boolean) => invoke<VersionList>("list_versions", { includeSnapshots }),
  /** Queue an install/repair for an install key (see Instance.installKey). */
  installVersion: (key: string) => invoke<number>("install_version", { id: key }),
  cancelDownload: (jobId: number) => invoke<void>("cancel_download", { jobId }),
  downloadQueue: () => invoke<QueueSnapshot>("download_queue"),
  downloadHistory: (limit: number) => invoke<DownloadRecord[]>("download_history", { limit }),
  clearDownloadHistory: () => invoke<number>("clear_download_history"),

  detectJava: (forVersion: string | null) => invoke<JavaRuntime[]>("detect_java", { forVersion }),
  setInstanceJava: (id: string, path: string | null) => invoke<JavaRuntime | null>("set_instance_java", { id, path }),

  listInstances: () => invoke<Instance[]>("list_instances"),
  getInstance: (id: string) => invoke<Instance>("get_instance", { id }),
  createInstance: (input: { name: string; gameVersion: string; loader: LoaderKind; loaderVersion: string | null; maxMemoryMb: number | null }) =>
    invoke<Instance>("create_instance", { input }),
  loaderVersions: (loader: LoaderKind, minecraft: string) => invoke<LoaderVersion[]>("loader_versions", { loader, minecraft }),
  setInstanceLoaderVersion: (id: string, version: string) => invoke<Instance>("set_instance_loader_version", { id, version }),
  setInstanceGameDir: (id: string, path: string | null) => invoke<Instance>("set_instance_game_dir", { id, path }),
  pickFolder: () => invoke<string | null>("pick_folder"),
  updateInstance: (id: string, patch: { name?: string; maxMemoryMb?: number | null }) =>
    invoke<Instance>("update_instance", { id, patch }),
  deleteInstance: (id: string) => invoke<{ trashPath: string | null }>("delete_instance", { id }),
  openInstanceDir: (id: string) => invoke<void>("open_instance_dir", { id }),
  setInstanceJvmArgs: (id: string, args: string) => invoke<Instance>("set_instance_jvm_args", { id, args }),

  account: () => invoke<Account | null>("account"),
  signIn: () => invoke<Account>("sign_in"),
  cancelSignIn: () => invoke<void>("cancel_sign_in"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  checkActiveAccount: () => invoke<Account | null>("check_active_account"),
  switchAccount: (uuid: string) => invoke<Account>("switch_account", { uuid }),
  removeAccount: (uuid: string) => invoke<Account | null>("remove_account", { uuid }),

  modrinthSearch: (instanceId: string, query: string, category: string | null, sort: SortIndex, offset: number) =>
    invoke<SearchView>("modrinth_search", { instanceId, query, category, sort, offset }),
  openModrinthPage: (slug: string) => invoke<void>("open_modrinth_page", { slug }),
  modrinthCategories: () => invoke<Category[]>("modrinth_categories"),
  modInstallPlan: (instanceId: string, projectId: string) => invoke<InstallPlan>("mod_install_plan", { instanceId, projectId }),
  modInstall: (instanceId: string, items: { versionId: string; requiredBy: string | null }[]) =>
    invoke<ModRecord[]>("mod_install", { instanceId, items }),
  listMods: (instanceId: string) => invoke<ModRecord[]>("list_mods", { instanceId }),
  setModEnabled: (instanceId: string, fileName: string, enabled: boolean) => invoke<void>("set_mod_enabled", { instanceId, fileName, enabled }),
  removeMod: (instanceId: string, fileName: string) => invoke<void>("remove_mod", { instanceId, fileName }),
  checkModUpdates: (instanceId: string) => invoke<ModUpdate[]>("check_mod_updates", { instanceId }),
  updateMods: (instanceId: string, fileNames: string[]) => invoke<number>("update_mods", { instanceId, fileNames }),
  importLocalMod: (instanceId: string) => invoke<{ record: ModRecord; warnings: string[] } | null>("import_local_mod", { instanceId }),

  runningInstances: () => invoke<Running[]>("running_instances"),
  launchInstance: (id: string) => invoke<void>("launch_instance", { id }),
  stopInstance: (id: string) => invoke<void>("stop_instance", { id }),
};

const on =
  <T,>(name: string) =>
  (cb: (payload: T) => void): Promise<UnlistenFn> =>
    listen<T>(name, (e) => cb(e.payload));

export const events = {
  installProgress: on<InstallProgress>("install://progress"),
  installFinished: on<InstallFinished>("install://finished"),
  downloadsQueue: on<QueueSnapshot>("downloads://queue"),
  gameStarted: on<string>("game://started"),
  gameLog: on<LogBatch>("game://log"),
  gameExit: on<GameExit>("game://exit"),
};
