import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Activity, Boxes, Compass, CornerDownLeft, Download, House, Play, Plus, Search, Settings, UserRound } from "lucide-react";
import { useApp } from "../stores/app";
import { navigate, type Route } from "../stores/router";
import { playInstance } from "../lib/actions";
import { toast } from "../stores/toast";

type Command = { id: string; label: string; hint?: string; icon: ReactNode; run: () => void; keywords?: string };

const NAV: { route: Route; label: string; icon: ReactNode; keywords?: string }[] = [
  { route: { name: "home" }, label: "Home", icon: <House size={15} /> },
  { route: { name: "instances" }, label: "Instances", icon: <Boxes size={15} /> },
  { route: { name: "new-instance" }, label: "New instance", icon: <Plus size={15} />, keywords: "create add" },
  { route: { name: "discover" }, label: "Discover mods", icon: <Compass size={15} />, keywords: "modrinth search browse" },
  { route: { name: "discover", kind: "resourcepack" }, label: "Discover resource packs", icon: <Compass size={15} />, keywords: "modrinth textures" },
  { route: { name: "discover", kind: "shader" }, label: "Discover shader packs", icon: <Compass size={15} />, keywords: "modrinth shaders iris" },
  { route: { name: "downloads" }, label: "Downloads", icon: <Download size={15} />, keywords: "queue history" },
  { route: { name: "accounts" }, label: "Accounts", icon: <UserRound size={15} />, keywords: "microsoft sign in login" },
  { route: { name: "settings" }, label: "Settings", icon: <Settings size={15} />, keywords: "preferences memory java" },
  { route: { name: "diagnostics" }, label: "Diagnostics", icon: <Activity size={15} />, keywords: "logs report debug" },
];

/** Every word of the query must start a word in the label or keywords. */
export function matches(query: string, text: string): boolean {
  const words = text.toLowerCase().split(/[^a-z0-9.]+/).filter(Boolean);
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((q) => words.some((w) => w.startsWith(q)));
}

/** Ctrl+K command palette: navigate, open or play instances. */
export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  // Select the stable store value and derive in a memo: returning a fresh
  // `[]` from the selector makes zustand v5 re-render forever (React #185).
  const loaded = useApp((s) => s.instances);
  const instances = useMemo(() => (loaded.status === "ready" ? loaded.data : []), [loaded]);
  const listId = useId();

  const commands = useMemo<Command[]>(() => {
    const nav = NAV.map((n) => ({ id: `nav:${n.route.name}${n.route.name === "discover" && n.route.kind ? `:${n.route.kind}` : ""}`, label: n.label, hint: "Go to", icon: n.icon, keywords: n.keywords, run: () => navigate(n.route) }));
    const inst = instances.flatMap((i) => [
      { id: `open:${i.id}`, label: i.name, hint: i.label, icon: <Boxes size={15} />, keywords: "open instance", run: () => navigate({ name: "instance", id: i.id }) },
      {
        id: `play:${i.id}`,
        label: `Play ${i.name}`,
        hint: i.label,
        icon: <Play size={15} />,
        keywords: "launch start",
        run: () => void playInstance(i).then((e) => e && toast({ tone: "error", title: `Could not start ${i.name}`, body: e.message })),
      },
    ]);
    return [...nav, ...inst];
  }, [instances]);

  const shown = useMemo(
    () => (query.trim() ? commands.filter((c) => matches(query, `${c.label} ${c.hint ?? ""} ${c.keywords ?? ""}`)) : commands).slice(0, 50),
    [commands, query],
  );

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      setQuery("");
      setActive(0);
      d.showModal();
      requestAnimationFrame(() => input.current?.focus());
    }
    if (!open && d.open) d.close();
  }, [open]);
  useEffect(() => setActive(0), [query]);

  function choose(c: Command | undefined) {
    if (!c) return;
    onClose();
    c.run();
  }

  return (
    <dialog
      ref={ref}
      onClose={onClose}
      onClick={(e) => e.target === ref.current && onClose()}
      aria-label="Command palette"
      className="mx-auto mt-[12vh] w-[560px] max-w-[calc(100vw-32px)] overflow-hidden rounded-lg border border-line-strong bg-surface-2 p-0 text-fg shadow-2xl backdrop:bg-black/50"
    >
      <div className="flex items-center gap-2 border-b border-line px-3">
        <Search size={15} className="shrink-0 text-fg-faint" />
        <input
          ref={input}
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={shown[active] ? `${listId}-${active}` : undefined}
          aria-label="Search commands"
          placeholder="Go to a screen, open or play an instance…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setActive((a) => Math.min(a + 1, shown.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setActive((a) => Math.max(a - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              choose(shown[active]);
            }
          }}
          className="h-12 flex-1 bg-transparent text-[14px] text-fg placeholder:text-fg-faint focus:outline-none"
        />
        <kbd className="rounded border border-line px-1.5 text-2xs text-fg-faint">Esc</kbd>
      </div>
      <ul id={listId} role="listbox" aria-label="Commands" className="max-h-[50vh] overflow-y-auto p-1.5">
        {shown.length === 0 && <li className="px-3 py-6 text-center text-[13px] text-fg-muted">No matching commands.</li>}
        {shown.map((c, i) => (
          <li
            key={c.id}
            id={`${listId}-${i}`}
            role="option"
            aria-selected={i === active}
            onMouseMove={() => setActive(i)}
            onClick={() => choose(c)}
            className={`flex cursor-pointer items-center gap-3 rounded-md px-3 py-2 text-[13px] ${i === active ? "bg-surface-3 text-fg" : "text-fg-muted"}`}
          >
            <span className={i === active ? "text-accent" : "text-fg-faint"}>{c.icon}</span>
            <span className="truncate">{c.label}</span>
            {c.hint && <span className="ml-auto truncate text-xs text-fg-faint">{c.hint}</span>}
            {i === active && <CornerDownLeft size={13} className="shrink-0 text-fg-faint" />}
          </li>
        ))}
      </ul>
    </dialog>
  );
}
