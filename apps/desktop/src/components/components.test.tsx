import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ProgressBar } from "./ProgressBar";
import { Button } from "./Button";
import { MAX_LOG_LINES, useLauncher } from "../stores/launcher";

afterEach(cleanup);

describe("ProgressBar", () => {
  it("exposes an accessible value", () => {
    render(<ProgressBar value={0.426} label="Install" />);
    const bar = screen.getByRole("progressbar", { name: "Install" });
    expect(bar.getAttribute("aria-valuenow")).toBe("43");
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

describe("launcher store", () => {
  it("caps retained log lines", () => {
    const batch = Array.from({ length: MAX_LOG_LINES + 50 }, (_, i) => ({ stream: "stdout" as const, level: null, text: `l${i}` }));
    useLauncher.getState().appendLogs(batch);
    const logs = useLauncher.getState().logs;
    expect(logs).toHaveLength(MAX_LOG_LINES);
    expect(logs.at(-1)?.text).toBe(`l${MAX_LOG_LINES + 49}`);
  });

  it("ignores progress for a different version and treats cancel as idle", () => {
    const s = useLauncher.getState();
    s.startInstall("26.3");
    s.setProgress({ id: "1.8.9", phase: "gameFiles", totalFiles: 1, doneFiles: 1, skippedFiles: 0, totalBytes: 1, doneBytes: 1 });
    expect(useLauncher.getState().install).toEqual({ status: "running", versionId: "26.3", progress: null });
    s.failInstall("26.3", { code: "cancelled", message: "" });
    expect(useLauncher.getState().install.status).toBe("idle");
  });
});
