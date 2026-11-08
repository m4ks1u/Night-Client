import type { ContentKind, LoaderKind as Loader } from "./ipc";

/** Wording and folders for each kind of Modrinth content. */
export const CONTENT: Record<ContentKind, { singular: string; plural: string; tab: string; folder: string }> = {
  mod: { singular: "mod", plural: "mods", tab: "Mods", folder: "mods" },
  resourcepack: { singular: "resource pack", plural: "resource packs", tab: "Resource packs", folder: "resourcepacks" },
  shader: { singular: "shader pack", plural: "shader packs", tab: "Shaders", folder: "shaderpacks" },
};

export const CONTENT_KINDS: ContentKind[] = ["mod", "resourcepack", "shader"];

/** Mods need a loader; packs work on every instance. */
export const kindAvailable = (kind: ContentKind, loader: Loader) => kind !== "mod" || loader !== "vanilla";

/** Modrinth tags that describe compatibility rather than content; hidden from result badges. */
export const COMPAT_TAGS = ["fabric", "neoforge", "forge", "quilt", "minecraft", "iris", "optifine", "vanilla", "canvas"];

/** How to use installed content in the game. */
export function usageHint(kind: ContentKind, loader: Loader): string | null {
  if (kind === "resourcepack") return "Turn resource packs on in the game under Options → Resource Packs.";
  if (kind === "shader")
    return loader === "vanilla"
      ? "Vanilla instances can only use core-shader packs, applied like resource packs. Most shader packs need a shader mod such as Iris, on a Fabric, Quilt or NeoForge instance."
      : "Shader packs need a shader mod, such as Iris (Fabric, Quilt, NeoForge) or OptiFine. Pick the pack in the shader mod's settings in game.";
  return null;
}
