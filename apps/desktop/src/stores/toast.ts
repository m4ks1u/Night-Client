import { create } from "zustand";
import type { Route } from "./router";

export type Toast = {
  id: number;
  tone: "success" | "error" | "info";
  title: string;
  body?: string;
  action?: { label: string; route: Route };
};

type ToastStore = {
  toasts: Toast[];
  push: (t: Omit<Toast, "id">) => number;
  dismiss: (id: number) => void;
};

let next = 1;
/** At most this many toasts are visible; older ones are dropped. */
export const MAX_TOASTS = 4;

export const useToasts = create<ToastStore>((set) => ({
  toasts: [],
  push: (t) => {
    const id = next++;
    set((s) => ({ toasts: [...s.toasts, { ...t, id }].slice(-MAX_TOASTS) }));
    return id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export const toast = (t: Omit<Toast, "id">) => useToasts.getState().push(t);
