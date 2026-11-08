import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ProgressBar } from "./ProgressBar";
import { Button } from "./Button";
import { EmptyState, MemorySlider, Toggle } from "./ui";
import { joinArgs } from "./JvmArgs";
import { MAX_LOG_LINES, effectiveMemory, useApp } from "../stores/app";
import { sectionOf, useRouter } from "../stores/router";
import { api, type Instance } from "../lib/ipc";

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
    const s = { defaultMaxMemoryMb: 6144, downloadConcurrency: 8, hideOnLaunch: false };
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
