import { create } from "zustand";
import type { ContentKind } from "../lib/ipc";

/** What Discover browses: content for an instance, or modpacks (new instances). */
export type DiscoverKind = ContentKind | "modpack";

export type InstanceTab = "overview" | "mods" | "resourcepacks" | "shaders" | "log";

export type Route =
  | { name: "home" }
  | { name: "instances" }
  | { name: "new-instance" }
  | { name: "instance"; id: string; tab?: InstanceTab }
  | { name: "discover"; instanceId?: string; kind?: DiscoverKind }
  | { name: "downloads" }
  | { name: "accounts" }
  | { name: "settings" }
  | { name: "diagnostics" };

type RouterStore = {
  route: Route;
  history: Route[];
  navigate: (r: Route) => void;
  back: () => void;
};

/** Minimal in-memory router: a desktop app has no URL bar, so no URL sync is needed. */
export const useRouter = create<RouterStore>((set) => ({
  route: { name: "home" },
  history: [],
  navigate: (r) => set((s) => ({ route: r, history: [...s.history.slice(-49), s.route] })),
  back: () =>
    set((s) => {
      const prev = s.history.at(-1);
      return prev ? { route: prev, history: s.history.slice(0, -1) } : s;
    }),
}));

export const navigate = (r: Route) => useRouter.getState().navigate(r);

/** Top-level section a route belongs to (for sidebar highlighting). */
export function sectionOf(r: Route): Route["name"] {
  return r.name === "instance" || r.name === "new-instance" ? "instances" : r.name;
}
