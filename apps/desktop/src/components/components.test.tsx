import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ProgressBar } from "./ProgressBar";
import { Button } from "./Button";
import { EmptyState, MemorySlider, Toggle } from "./ui";
import { joinArgs } from "./JvmArgs";
import { labelForKey } from "../lib/keys";
import { MAX_LOG_LINES, effectiveMemory, useApp } from "../stores/app";
import { sectionOf, useRouter } from "../stores/router";
import { api, type Instance, type PlannedMod } from "../lib/ipc";
import { itemsToInstall } from "./ModParts";

afterEach(cleanup);

const inst = (over: Partial<Instance> = {}): Instance => ({
  id: "3f2b8c1e-0000-4000-8000-123456789abc",
  name: "World",
  dirName: "World",
  gameVersion: "1.21.1",
  loader: "vanilla",
  loaderVersion: null,
  maxMemoryMb: null,
  javaPath: null,
  jvmArgs: [],
  gameDir: null,
  installKey: "vanilla:1.21.1",
  label: "Minecraft 1.21.1",
  createdAt: 0,
  lastPlayedAt: null,
  playTimeSeconds: 0,
  installed: true,
  running: false,
  ...over,
});

describe("ProgressBar", () => {
  it("exposes an accessible value", () => {
    render(<ProgressBar value={0.426} label="Install" />);
    expect(screen.getByRole("progressbar", { name: "Install" }).getAttribute("aria-valuenow")).toBe("43");
  });
  it("is indeterminate without a value", () => {
    render(<ProgressBar value={null} label="Install" />);
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBeNull();
  });
});

describe("Button", () => {
  it("defaults to type=button so it never submits forms", () => {
    render(<Button>Go</Button>);
    expect(screen.getByRole("button", { name: "Go" }).getAttribute("type")).toBe("button");
  });
});

describe("Toggle", () => {
  it("is a labelled switch that flips on click", () => {
    const onChange = vi.fn();
    render(<Toggle id="t" checked={false} onChange={onChange} label="Hide while playing" />);
    const sw = screen.getByRole("switch", { name: "Hide while playing" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(sw);
    expect(onChange).toHaveBeenCalledWith(true);
  });
});

describe("MemorySlider", () => {
  it("shows the recommendation and warns when far above it", () => {
    const { rerender } = render(<MemorySlider value={4096} onChange={() => {}} max={14336} recommended={4096} />);
    expect(screen.getByText("Recommended for this PC: 4 GB")).toBeTruthy();
    rerender(<MemorySlider value={12288} onChange={() => {}} max={14336} recommended={4096} />);
    expect(screen.getByText(/garbage-collection pauses/)).toBeTruthy();
  });
  it("never offers more than the host maximum", () => {
    render(<MemorySlider value={4096} onChange={() => {}} max={6144} />);
    expect(screen.getByRole("slider").getAttribute("max")).toBe("6144");
  });
});

describe("EmptyState", () => {
  it("renders its call to action", () => {
    render(<EmptyState icon={null} title="No instances yet" body="b" action={<Button>New instance</Button>} />);
    expect(screen.getByRole("heading", { name: "No instances yet" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "New instance" })).toBeTruthy();
  });
});

describe("app store", () => {
  beforeEach(() =>
    useApp.setState({ logs: {}, running: {}, queue: { active: null, pending: [] }, progress: null, installErrors: {}, lastFinished: {}, instances: { status: "ready", data: [] } }),
  );

  it("caps retained log lines per instance", () => {
    const batch = Array.from({ length: MAX_LOG_LINES + 50 }, (_, i) => ({ stream: "stdout" as const, level: null, text: `l${i}` }));
    useApp.getState().appendLogs("a", batch);
    useApp.getState().appendLogs("b", batch.slice(0, 3));
    expect(useApp.getState().logs.a).toHaveLength(MAX_LOG_LINES);
    expect(useApp.getState().logs.a?.at(-1)?.text).toBe(`l${MAX_LOG_LINES + 49}`);
    expect(useApp.getState().logs.b).toHaveLength(3);
  });

  it("only applies progress for the active job", () => {
    useApp.getState().setQueue({ active: { id: 7, kind: "version", target: "26.3", queuedAt: 0 }, pending: [] });
    useApp.getState().setProgress({ jobId: 3, id: "1.8.9", phase: "gameFiles", totalFiles: 1, doneFiles: 1, skippedFiles: 0, totalBytes: 1, doneBytes: 1 });
    expect(useApp.getState().progress).toBeNull();
    useApp.getState().setProgress({ jobId: 7, id: "26.3", phase: "runtime", totalFiles: 2, doneFiles: 1, skippedFiles: 0, totalBytes: 10, doneBytes: 5 });
    expect(useApp.getState().progress?.phase).toBe("runtime");
    useApp.getState().setQueue({ active: null, pending: [] });
    expect(useApp.getState().progress).toBeNull();
  });

  it("installVersion resolves when its job finishes and records failures", async () => {
    vi.spyOn(api, "installVersion").mockResolvedValue(1);
    vi.spyOn(api, "listInstances").mockResolvedValue([]);
    const ok = useApp.getState().installVersion("1.21.1");
    const failed = useApp.getState().installVersion("26.3");
    await Promise.resolve();
    const base = { jobId: 1, code: null, error: null, repair: true, repairedCount: 2 };
    useApp.getState().onFinished({ ...base, id: "1.21.1", status: "completed" });
    useApp.getState().onFinished({ ...base, jobId: 2, id: "26.3", status: "failed", code: "network", error: "offline" });
    expect(await ok).toBe(true);
    expect(await failed).toBe(false);
    expect(useApp.getState().installErrors["26.3"]).toEqual({ code: "network", message: "offline" });
    expect(useApp.getState().lastFinished["1.21.1"]?.repairedCount).toBe(2);
  });

  it("upserts and removes instances", () => {
    useApp.getState().upsertInstance(inst());
    useApp.getState().upsertInstance(inst({ name: "Renamed" }));
    const s = useApp.getState().instances;
    expect(s.status === "ready" && s.data.map((i) => i.name)).toEqual(["Renamed"]);
    useApp.getState().removeInstance(inst().id);
    const after = useApp.getState().instances;
    expect(after.status === "ready" && after.data).toEqual([]);
  });

  it("tracks running games", () => {
    useApp.getState().markStarted("x");
    expect("x" in useApp.getState().running).toBe(true);
    useApp.getState().markExited("x");
    expect("x" in useApp.getState().running).toBe(false);
  });

  it("uses the instance override or the launcher default for memory", () => {
    const s = { defaultMaxMemoryMb: 6144, downloadConcurrency: 8, hideOnLaunch: false, reduceMotion: false, compactLayout: false, gpuAcceleration: false, checkUpdates: true, onboarded: true };
    expect(effectiveMemory(inst(), s)).toBe(6144);
    expect(effectiveMemory(inst({ maxMemoryMb: 2048 }), s)).toBe(2048);
    expect(effectiveMemory(inst(), null)).toBe(4096);
  });
});

describe("router", () => {
  it("navigates, goes back, and maps sections", () => {
    useRouter.setState({ route: { name: "home" }, history: [] });
    useRouter.getState().navigate({ name: "instances" });
    useRouter.getState().navigate({ name: "instance", id: "x" });
    expect(sectionOf(useRouter.getState().route)).toBe("instances");
    useRouter.getState().back();
    expect(useRouter.getState().route).toEqual({ name: "instances" });
    useRouter.getState().back();
    useRouter.getState().back(); // no-op at the start of history
    expect(useRouter.getState().route).toEqual({ name: "home" });
  });
});

describe("joinArgs", () => {
  it("quotes arguments the backend splitter would otherwise break up", () => {
    expect(joinArgs(["-XX:+UseG1GC", "-Dname=hello world", 'q"x', ""])).toBe('-XX:+UseG1GC "-Dname=hello world" "q\\"x" ""');
    expect(joinArgs([])).toBe("");
  });
});

describe("labelForKey", () => {
  it("labels every key shape, including pre-Stage-4 bare versions", () => {
    expect(labelForKey("vanilla:1.21.1")).toBe("Minecraft 1.21.1");
    expect(labelForKey("fabric:1.21.1:0.17.2")).toBe("Fabric 0.17.2 · 1.21.1");
    expect(labelForKey("quilt:1.21.1:0.29.2")).toBe("Quilt 0.29.2 · 1.21.1");
    expect(labelForKey("neoforge:26.3:26.3.0.101")).toBe("NeoForge 26.3.0.101 · 26.3");
    expect(labelForKey("1.12.2")).toBe("Minecraft 1.12.2");
  });
});

import { matches } from "./CommandPalette";
import { hash32 } from "./InstanceArt";
import { MAX_TOASTS, useToasts } from "../stores/toast";
import { Toaster } from "./Toaster";
import { LazyMotion, domAnimation } from "motion/react";

describe("command palette matching", () => {
  it("matches word starts in any order", () => {
    expect(matches("dis", "Discover mods Go to modrinth search")).toBe(true);
    expect(matches("play surv", "Play Survival World Fabric 0.20.1 · 1.21.1")).toBe(true);
    expect(matches("1.21", "Play Survival Fabric 0.20.1 · 1.21.1")).toBe(true);
    expect(matches("over", "Discover mods")).toBe(false);
    expect(matches("", "anything")).toBe(true);
  });
});

describe("instance art", () => {
  it("is deterministic per instance and differs between instances", () => {
    expect(hash32("a")).toBe(hash32("a"));
    expect(hash32("3f2b8c1e-0000")).not.toBe(hash32("3f2b8c1e-0001"));
  });
});

describe("toasts", () => {
  beforeEach(() => useToasts.setState({ toasts: [] }));

  it("keeps at most MAX_TOASTS and dismisses by id", () => {
    for (let i = 0; i < MAX_TOASTS + 2; i++) useToasts.getState().push({ tone: "info", title: `t${i}` });
    const t = useToasts.getState().toasts;
    expect(t).toHaveLength(MAX_TOASTS);
    expect(t[0]?.title).toBe("t2");
    useToasts.getState().dismiss(t[0]!.id);
    expect(useToasts.getState().toasts).toHaveLength(MAX_TOASTS - 1);
  });

  it("renders errors as alerts with an action that navigates and dismisses", () => {
    useRouter.setState({ route: { name: "home" }, history: [] });
    useToasts.getState().push({ tone: "error", title: "Install failed", body: "network", action: { label: "View downloads", route: { name: "downloads" } } });
    render(
      <LazyMotion features={domAnimation}>
        <Toaster />
      </LazyMotion>,
    );
    expect(screen.getByRole("alert").textContent).toContain("Install failed");
    fireEvent.click(screen.getByRole("button", { name: "View downloads" }));
    expect(useRouter.getState().route).toEqual({ name: "downloads" });
    expect(useToasts.getState().toasts).toHaveLength(0);
  });
});

import { CommandPalette } from "./CommandPalette";
import { updateNotices } from "./ModsTab";
import { targetFor } from "./DataFolderCard";

describe("data folder target", () => {
  it("puts the data in a Night Client subfolder of the picked folder", () => {
    expect(targetFor("D:\\Games")).toBe("D:\\Games\\Night Client");
    expect(targetFor("D:\\")).toBe("D:\\Night Client");
    expect(targetFor("E:\\Night Client\\")).toBe("E:\\Night Client");
  });
});

describe("mod update notices", () => {
  it("reports added and missing dependencies, and nothing otherwise", () => {
    expect(updateNotices({ updated: 2, addedDependencies: [], missingDependencies: [] })).toEqual([]);
    const n = updateNotices({ updated: 1, addedDependencies: ["Fabric API"], missingDependencies: ["Cloth Config"] });
    expect(n).toHaveLength(2);
    expect(n[0]).toContain("Fabric API");
    expect(n[1]).toContain("Cloth Config");
  });
});

describe("command palette render", () => {
  it("renders while instances are still loading (regression: React #185 update loop)", () => {
    useApp.setState({ instances: { status: "idle" } });
    expect(() => render(<CommandPalette open={false} onClose={() => {}} />)).not.toThrow();
    useApp.setState({ instances: { status: "loading" } });
    useApp.setState({ instances: { status: "ready", data: [] } });
  });
});

describe("itemsToInstall", () => {
  const mod = (project: string, requiredBy: string | null = null): PlannedMod => ({
    version: { id: `${project}-v`, project_id: project, name: project, version_number: "1", version_type: "release", files: [{ filename: `${project}.jar`, size: 1, primary: true }] },
    requiredBy,
  });
  const plan = (install: PlannedMod[]) => ({ install, alreadyInstalled: [], missing: [], conflicts: [], optional: [] });
  const full = {
    ...plan([mod("A"), mod("B", "A")]),
    optional: ["C", "D"],
    // C needs B (already planned) and E; D needs nothing.
    options: [plan([mod("C"), mod("B", "C"), mod("E", "C")]), plan([mod("D")])],
    titles: {},
  };
  it("installs only the main plan when nothing optional is chosen", () => {
    expect(itemsToInstall(full, new Set()).map((m) => m.version.project_id)).toEqual(["A", "B"]);
  });
  it("adds a chosen option with its dependencies, each project once", () => {
    expect(itemsToInstall(full, new Set(["C"])).map((m) => m.version.project_id)).toEqual(["A", "B", "C", "E"]);
    expect(itemsToInstall(full, new Set(["C", "D"])).map((m) => m.version.project_id)).toEqual(["A", "B", "C", "E", "D"]);
  });
});
