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
  /** Whether this run draws with the GPU (the setting applies at startup). */
  gpuActive: boolean;
};
export type SystemInfo = {
  totalMemoryMb: number;
  availableMemoryMb: number;
  osVersion: string;
  cpuThreads: number;
  recommendedMemoryMb: number;
  maxMemoryMb: number;
};
export type Settings = { defaultMaxMemoryMb: number; downloadConcurrency: number; hideOnLaunch: boolean; reduceMotion: boolean; compactLayout: boolean; gpuAcceleration: boolean; checkUpdates: boolean; onboarded: boolean };
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
export type LoaderKind = "vanilla" | "fabric" | "quilt" | "neoforge" | "forge";
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
export type ModpackHit = Omit<SearchHit, "installed">;
export type ModpackHits = { hits: ModpackHit[]; total: number; offset: number };
/** A validated .mrpack held by the backend behind `token`. */
export type PackPreview = {
  token: string;
  name: string;
  versionId: string;
  summary: string | null;
  minecraft: string;
  loader: LoaderKind;
  loaderVersion: string | null;
  label: string;
  files: number;
  optionalFiles: number;
  skipped: number;
  downloadBytes: number;
  iconUrl: string | null;
};
export type ModpackProgress = {
  token: string;
  phase: "files" | "overrides";
  doneFiles: number;
  totalFiles: number;
  doneBytes: number;
  totalBytes: number;
};
export type Category = { name: string; project_type: string; header: string };
export type MrVersion = {
  id: string;
  project_id: string;
  name: string;
  version_number: string;
  version_type: string;
  files: { filename: string; size: number; primary: boolean }[];
};
export type PlannedMod = { version: MrVersion; requiredBy: string | null };
export type Plan = {
  install: PlannedMod[];
  alreadyInstalled: string[];
  missing: string[];
  conflicts: string[];
  /** Optional dependencies of the chosen mod (project ids). */
  optional: string[];
};
export type InstallPlan = Plan & {
  /** One plan per installable optional dependency: that mod first, then what it needs. */
  options: Plan[];
  titles: Record<string, string>;
};
/** Kinds of Modrinth content: each has its own folder in the game directory. */
export type ContentKind = "mod" | "resourcepack" | "shader";
export type ModRecord = {
  instanceId: string;
  kind: ContentKind;
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
export type UpdateInfo = {
  /** False for builds without an update channel (development builds). */
  configured: boolean;
  currentVersion: string;
  available: { version: string; notes: string; date: string | null } | null;
};
export type DataFolder = { path: string; defaultPath: string; isDefault: boolean; sizeBytes: number };
export type UpdateSummary = { updated: number; addedDependencies: string[]; missingDependencies: string[] };
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
  source: "managed" | "javaHome" | "path" | "programFiles" | "registry" | "minecraftLauncher" | "custom";
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
  dataFolderInfo: () => invoke<DataFolder>("data_folder_info"),
  /** Moves the game data (`null` = back to the default folder) and restarts the app on success. */
  relocateData: (target: string | null) => invoke<void>("relocate_data", { target }),
  checkForUpdate: () => invoke<UpdateInfo>("check_for_update"),
  /** Downloads, verifies and runs the installer; Night Client closes and restarts on success. */
  installUpdate: () => invoke<void>("install_update"),
  takeCrashNotice: () => invoke<{ count: number; newestUnix: number | null }>("take_crash_notice"),
  /** Saves a redacted zip through a native save dialog; `null` if cancelled. */
  exportDiagnostics: () => invoke<string | null>("export_diagnostics"),
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

  // Content commands take an optional kind; the backend defaults to mods.
  modrinthSearch: (instanceId: string, query: string, category: string | null, sort: SortIndex, offset: number, kind: ContentKind = "mod") =>
    invoke<SearchView>("modrinth_search", { instanceId, query, category, sort, offset, kind }),
  openModrinthPage: (slug: string, kind: ContentKind = "mod") => invoke<void>("open_modrinth_page", { slug, kind }),
  openModpackPage: (slug: string) => invoke<void>("open_modrinth_page", { slug, modpack: true }),
  modpackSearch: (query: string, sort: SortIndex, offset: number) => invoke<ModpackHits>("modpack_search", { query, sort, offset }),
  /** Native file picker; `null` when cancelled. */
  modpackPreviewFile: () => invoke<PackPreview | null>("modpack_preview_file"),
  modpackPreviewModrinth: (projectId: string) => invoke<PackPreview>("modpack_preview_modrinth", { projectId }),
  modpackImport: (token: string, name: string) => invoke<Instance>("modpack_import", { token, name }),
  modpackCancel: (token: string) => invoke<void>("modpack_cancel", { token }),
  modrinthCategories: (kind: ContentKind = "mod") => invoke<Category[]>("modrinth_categories", { kind }),
  modInstallPlan: (instanceId: string, projectId: string, kind: ContentKind = "mod") =>
    invoke<InstallPlan>("mod_install_plan", { instanceId, projectId, kind }),
  modInstall: (instanceId: string, items: { versionId: string; requiredBy: string | null }[], kind: ContentKind = "mod") =>
    invoke<ModRecord[]>("mod_install", { instanceId, items, kind }),
  listMods: (instanceId: string, kind: ContentKind = "mod") => invoke<ModRecord[]>("list_mods", { instanceId, kind }),
  setModEnabled: (instanceId: string, fileName: string, enabled: boolean, kind: ContentKind = "mod") =>
    invoke<void>("set_mod_enabled", { instanceId, fileName, enabled, kind }),
  removeMod: (instanceId: string, fileName: string, kind: ContentKind = "mod") => invoke<void>("remove_mod", { instanceId, fileName, kind }),
  checkModUpdates: (instanceId: string, kind: ContentKind = "mod") => invoke<ModUpdate[]>("check_mod_updates", { instanceId, kind }),
  updateMods: (instanceId: string, fileNames: string[], kind: ContentKind = "mod") =>
    invoke<UpdateSummary>("update_mods", { instanceId, fileNames, kind }),
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
  relocateProgress: on<{ doneBytes: number; totalBytes: number }>("data://relocate-progress"),
  updateAvailable: on<string>("update://available"),
  updateProgress: on<{ doneBytes: number; totalBytes: number | null }>("update://progress"),
  modpackProgress: on<ModpackProgress>("modpack://progress"),
};
