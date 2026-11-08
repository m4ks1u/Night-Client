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
export type Settings = { defaultMaxMemoryMb: number; downloadConcurrency: number; hideOnLaunch: boolean };
export type VersionItem = { id: string; kind: "release" | "snapshot" | string; releaseTime: string; installed: boolean };
export type VersionList = { latestRelease: string; latestSnapshot: string; versions: VersionItem[]; fromCache: boolean };
export type Account = { name: string; uuid: string; expiresAt: number };

export type Instance = {
  id: string;
  name: string;
  dirName: string;
  gameVersion: string;
  loader: "vanilla" | "fabric" | "neoforge";
  loaderVersion: string | null;
  maxMemoryMb: number | null;
  javaPath: string | null;
  createdAt: number;
  lastPlayedAt: number | null;
  playTimeSeconds: number;
  installed: boolean;
  running: boolean;
};
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

export type InstallPhase = "metadata" | "runtime" | "gameFiles";
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
  openLogsDir: () => invoke<void>("open_logs_dir"),

  listVersions: (includeSnapshots: boolean) => invoke<VersionList>("list_versions", { includeSnapshots }),
  installVersion: (id: string) => invoke<number>("install_version", { id }),
  cancelDownload: (jobId: number) => invoke<void>("cancel_download", { jobId }),
  downloadQueue: () => invoke<QueueSnapshot>("download_queue"),
  downloadHistory: (limit: number) => invoke<DownloadRecord[]>("download_history", { limit }),
  clearDownloadHistory: () => invoke<number>("clear_download_history"),

  detectJava: (forVersion: string | null) => invoke<JavaRuntime[]>("detect_java", { forVersion }),
  setInstanceJava: (id: string, path: string | null) => invoke<JavaRuntime | null>("set_instance_java", { id, path }),

  listInstances: () => invoke<Instance[]>("list_instances"),
  getInstance: (id: string) => invoke<Instance>("get_instance", { id }),
  createInstance: (input: { name: string; gameVersion: string; maxMemoryMb: number | null }) =>
    invoke<Instance>("create_instance", { input }),
  updateInstance: (id: string, patch: { name?: string; maxMemoryMb?: number | null }) =>
    invoke<Instance>("update_instance", { id, patch }),
  deleteInstance: (id: string) => invoke<{ trashPath: string | null }>("delete_instance", { id }),
  openInstanceDir: (id: string) => invoke<void>("open_instance_dir", { id }),

  account: () => invoke<Account | null>("account"),
  signIn: () => invoke<Account>("sign_in"),
  cancelSignIn: () => invoke<void>("cancel_sign_in"),
  signOut: () => invoke<void>("sign_out"),

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
