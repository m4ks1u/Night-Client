import { describe, expect, it } from "vitest";
import { formatBytes, formatMemory, fraction } from "./format";
import { toCmdError } from "./ipc";

describe("formatBytes", () => {
  it("formats binary units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(614_384_188)).toBe("585.9 MB");
  });
  it("rejects nonsense", () => {
    expect(formatBytes(-1)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
  });
});

describe("fraction", () => {
  it("prefers bytes, falls back to files, clamps", () => {
    expect(fraction({ doneBytes: 50, totalBytes: 100, doneFiles: 0, totalFiles: 10 })).toBe(0.5);
    expect(fraction({ doneBytes: 0, totalBytes: 0, doneFiles: 3, totalFiles: 4 })).toBe(0.75);
    expect(fraction({ doneBytes: 200, totalBytes: 100, doneFiles: 0, totalFiles: 0 })).toBe(1);
    expect(fraction({ doneBytes: 0, totalBytes: 0, doneFiles: 0, totalFiles: 0 })).toBe(0);
  });
});

describe("formatMemory", () => {
  it("shows GB", () => {
    expect(formatMemory(4096)).toBe("4 GB");
    expect(formatMemory(1536)).toBe("1.5 GB");
  });
});

describe("toCmdError", () => {
  it("passes through backend errors and wraps others", () => {
    expect(toCmdError({ code: "network", message: "x" })).toEqual({ code: "network", message: "x" });
    expect(toCmdError(new Error("boom"))).toEqual({ code: "unknown", message: "boom" });
    expect(toCmdError("plain")).toEqual({ code: "unknown", message: "plain" });
  });
});
