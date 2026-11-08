// Install keys mirror nc_core::loader::InstallKey: "vanilla:<mc>", "fabric:<mc>:<v>", "quilt:<mc>:<v>", "neoforge:<mc>:<v>".

export function labelForKey(key: string): string {
  const [kind, mc, v] = key.split(":");
  if (kind === "fabric" && v) return `Fabric ${v} · ${mc}`;
  if (kind === "quilt" && v) return `Quilt ${v} · ${mc}`;
  if (kind === "neoforge" && v) return `NeoForge ${v} · ${mc}`;
  if (kind === "vanilla" && mc) return `Minecraft ${mc}`;
  // Rows recorded before Stage 4 stored the bare version id.
  return `Minecraft ${key}`;
}

export const LOADER_NAME = { vanilla: "Vanilla", fabric: "Fabric", quilt: "Quilt", neoforge: "NeoForge" } as const;
