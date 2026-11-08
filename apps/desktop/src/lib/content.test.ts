import { describe, expect, it } from "vitest";
import { CONTENT, CONTENT_KINDS, kindAvailable, usageHint } from "./content";

describe("content kinds", () => {
  it("names a folder per kind, matching the backend", () => {
    expect(CONTENT_KINDS.map((k) => CONTENT[k].folder)).toEqual(["mods", "resourcepacks", "shaderpacks"]);
  });

  it("offers mods only with a loader, packs everywhere", () => {
    expect(kindAvailable("mod", "vanilla")).toBe(false);
    expect(kindAvailable("mod", "quilt")).toBe(true);
    expect(kindAvailable("resourcepack", "vanilla")).toBe(true);
    expect(kindAvailable("shader", "vanilla")).toBe(true);
  });

  it("explains how to use packs, and the shader mod requirement", () => {
    expect(usageHint("mod", "fabric")).toBeNull();
    expect(usageHint("resourcepack", "vanilla")).toMatch(/Options → Resource Packs/);
    expect(usageHint("shader", "vanilla")).toMatch(/core-shader/);
    expect(usageHint("shader", "fabric")).toMatch(/Iris/);
  });
});
