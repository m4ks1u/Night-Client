// Typed bridge to the Rust commands in src-tauri/src/commands.rs.
// Keep this file the only place that names IPC commands and events.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type CmdError = { code: string; message: string };

export function toCmdError(e: unknown): CmdError {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) {
    return { code: String(e.code), message: String(e.message) };
  }
  return { code: "unknown", message: e instanceof Error ? e.message : String(e) };
}

export type AppInfo = { version: string; dataDir: string; signInConfigured: boolean };
export type VersionItem = { id: string; kind: string; releaseTime: string; installed: boolean };
export type VersionList = { latestRelease: string; versions: VersionItem[]; fromCache: boolean };
export type InstallResult = { downloadedFiles: number; skippedFiles: number; downloadedBytes: number };
export type Account = { name: string; uuid: string };

export type InstallPhase = "metadata" | "runtime" | "gameFiles";
export type InstallProgress = {
  id: string;
  phase: InstallPhase;
  totalFiles: number;
  doneFiles: number;
  skippedFiles: number;
  totalBytes: number;
  doneBytes: number;
};
export type LogLine = { stream: "stdout" | "stderr"; level: string | null; text: string };
export type GameExit = { version: string; code: number | null };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listVersions: () => invoke<VersionList>("list_versions"),
  installVersion: (id: string) => invoke<InstallResult>("install_version", { id }),
  cancelInstall: () => invoke<void>("cancel_install"),
  account: () => invoke<Account | null>("account"),
  signIn: () => invoke<Account>("sign_in"),
  cancelSignIn: () => invoke<void>("cancel_sign_in"),
  signOut: () => invoke<void>("sign_out"),
  launchGame: (id: string, maxMemoryMb: number) => invoke<void>("launch_game", { id, maxMemoryMb }),
  stopGame: () => invoke<void>("stop_game"),
  openDataDir: () => invoke<void>("open_data_dir"),
};

export const events = {
  onInstallProgress: (cb: (p: InstallProgress) => void): Promise<UnlistenFn> =>
    listen<InstallProgress>("install://progress", (e) => cb(e.payload)),
  onGameLog: (cb: (lines: LogLine[]) => void): Promise<UnlistenFn> =>
    listen<LogLine[]>("game://log", (e) => cb(e.payload)),
  onGameExit: (cb: (e: GameExit) => void): Promise<UnlistenFn> =>
    listen<GameExit>("game://exit", (e) => cb(e.payload)),
};
