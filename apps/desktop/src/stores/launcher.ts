import { create } from "zustand";
import type { CmdError, InstallProgress, InstallResult, LogLine } from "../lib/ipc";

export const MAX_LOG_LINES = 2000;

type InstallState =
  | { status: "idle" }
  | { status: "running"; versionId: string; progress: InstallProgress | null }
  | { status: "done"; versionId: string; result: InstallResult }
  | { status: "error"; versionId: string; error: CmdError };

type GameState = { status: "stopped" | "starting" | "running"; lastExit: number | null | undefined };

type LauncherStore = {
  install: InstallState;
  game: GameState;
  logs: LogLine[];
  startInstall: (versionId: string) => void;
  setProgress: (p: InstallProgress) => void;
  finishInstall: (versionId: string, r: InstallResult) => void;
  failInstall: (versionId: string, e: CmdError) => void;
  setGame: (g: Partial<GameState>) => void;
  appendLogs: (lines: LogLine[]) => void;
  clearLogs: () => void;
};

export const useLauncher = create<LauncherStore>((set) => ({
  install: { status: "idle" },
  game: { status: "stopped", lastExit: undefined },
  logs: [],
  startInstall: (versionId) => set({ install: { status: "running", versionId, progress: null } }),
  setProgress: (p) =>
    set((s) =>
      s.install.status === "running" && s.install.versionId === p.id
        ? { install: { ...s.install, progress: p } }
        : s,
    ),
  finishInstall: (versionId, result) => set({ install: { status: "done", versionId, result } }),
  failInstall: (versionId, error) =>
    set({ install: error.code === "cancelled" ? { status: "idle" } : { status: "error", versionId, error } }),
  setGame: (g) => set((s) => ({ game: { ...s.game, ...g } })),
  appendLogs: (lines) =>
    set((s) => {
      const next = s.logs.length + lines.length > MAX_LOG_LINES ? [...s.logs, ...lines].slice(-MAX_LOG_LINES) : [...s.logs, ...lines];
      return { logs: next };
    }),
  clearLogs: () => set({ logs: [] }),
}));
