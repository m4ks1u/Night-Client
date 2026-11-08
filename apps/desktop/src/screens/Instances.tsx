import { useMemo, useState } from "react";
import { Boxes, LayoutGrid, List, Plus, Search } from "lucide-react";
import { Button } from "../components/Button";
import { PlayButton } from "../components/InstanceActions";
import { InstanceArt } from "../components/InstanceArt";
import { Badge, EmptyState, ErrorState, PageHeader, Skeleton, inputClass } from "../components/ui";
import { formatDuration, formatRelative } from "../lib/format";
import type { Instance } from "../lib/ipc";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

type View = "grid" | "list";
const VIEW_KEY = "nc.instances.view";

/** Per-viewer convenience only; storage may be unavailable. */
function loadView(): View {
  try {
    return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

function saveView(v: View) {
  try {
    localStorage.setItem(VIEW_KEY, v);
  } catch {
    /* ignore */
  }
}

function StatusBadge({ i, running }: { i: Instance; running: boolean }) {
  if (running) return <Badge tone="success">Running</Badge>;
  if (!i.installed) return <Badge tone="warning">Not installed</Badge>;
  return null;
}

export function Instances() {
  const instances = useApp((s) => s.instances);
  const running = useApp((s) => s.running);
  const refresh = useApp((s) => s.refreshInstances);
  const [query, setQuery] = useState("");
  const [view, setView] = useState<View>(loadView);

  const filtered = useMemo(() => {
    if (instances.status !== "ready") return [];
    const q = query.trim().toLowerCase();
    return q ? instances.data.filter((i) => i.name.toLowerCase().includes(q) || i.label.toLowerCase().includes(q)) : instances.data;
  }, [instances, query]);

  const hasAny = instances.status === "ready" && instances.data.length > 0;
  const switchView = (v: View) => {
    setView(v);
    saveView(v);
  };

  return (
    <div>
      <PageHeader
        title="Instances"
        subtitle="Each instance has its own game folder, worlds and settings."
        actions={
          <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })} aria-keyshortcuts="Control+N">
            New instance
          </Button>
        }
      />

      {hasAny && (
        <div className="mb-4 flex items-center gap-3">
          <div className="relative max-w-sm flex-1">
            <Search size={14} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-fg-faint" />
            <input
              type="search"
              aria-label="Search instances"
              placeholder="Search by name, loader or version"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className={`${inputClass} pl-8`}
            />
          </div>
          <div role="radiogroup" aria-label="Layout" className="ml-auto inline-flex rounded-md border border-line bg-surface-2 p-0.5">
            {(
              [
                ["grid", <LayoutGrid key="g" size={14} />, "Grid"],
                ["list", <List key="l" size={14} />, "List"],
              ] as const
            ).map(([v, icon, label]) => (
              <button
                key={v}
                type="button"
                role="radio"
                aria-checked={view === v}
                aria-label={label}
                onClick={() => switchView(v)}
                className={`grid h-7 w-8 place-items-center rounded-[7px] transition-colors duration-(--duration-fast) ${view === v ? "bg-surface-3 text-fg" : "text-fg-faint hover:text-fg"}`}
              >
                {icon}
              </button>
            ))}
          </div>
        </div>
      )}

      {instances.status === "error" ? (
        <ErrorState title="Instances could not be loaded" error={instances.error} onRetry={() => void refresh()} />
      ) : instances.status !== "ready" ? (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-4">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-56" />
          ))}
        </div>
      ) : !hasAny ? (
        <EmptyState
          icon={<Boxes size={20} />}
          title="No instances yet"
          body="Create an instance to choose a Minecraft version, a mod loader if you want one, and start playing."
          action={
            <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })}>
              New instance
            </Button>
          }
        />
      ) : filtered.length === 0 ? (
        <p className="py-10 text-center text-[13px] text-fg-muted">No instances match “{query}”.</p>
      ) : view === "grid" ? (
        <ul className="grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-4" aria-label="Instances">
          {filtered.map((i) => (
            <li key={i.id}>
              <article className="group flex h-full flex-col overflow-hidden rounded-lg border border-line bg-surface-1 transition-colors duration-(--duration-fast) hover:border-line-strong">
                <button type="button" className="text-left" onClick={() => navigate({ name: "instance", id: i.id })} aria-label={`Open ${i.name}`}>
                  <InstanceArt seed={i.id} loader={i.loader} name={i.name} className="aspect-[16/8] w-full" />
                </button>
                <div className="flex flex-1 flex-col gap-3 p-4">
                  <button type="button" className="min-w-0 text-left" onClick={() => navigate({ name: "instance", id: i.id })}>
                    <span className="flex items-center gap-2">
                      <span className="truncate text-[14px] font-semibold group-hover:text-accent-hover">{i.name}</span>
                      <StatusBadge i={i} running={i.id in running} />
                    </span>
                    <span className="mt-0.5 block truncate text-xs text-fg-faint">
                      {i.label} · {formatRelative(i.lastPlayedAt)}
                    </span>
                  </button>
                  <div className="mt-auto">
                    <PlayButton instance={i} size="sm" showProgress={false} />
                  </div>
                </div>
              </article>
            </li>
          ))}
        </ul>
      ) : (
        <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1" aria-label="Instances">
          {filtered.map((i) => (
            <li key={i.id} className="flex items-center gap-4 px-4 py-3 transition-colors hover:bg-surface-2/60">
              <button type="button" className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => navigate({ name: "instance", id: i.id })}>
                <InstanceArt seed={i.id} loader={i.loader} name={i.name} className="h-10 w-16 shrink-0 overflow-hidden rounded-md" />
                <span className="min-w-0">
                  <span className="flex items-center gap-2">
                    <span className="truncate text-[13px] font-medium">{i.name}</span>
                    <StatusBadge i={i} running={i.id in running} />
                  </span>
                  <span className="text-xs text-fg-faint">
                    {i.label} · Last played {formatRelative(i.lastPlayedAt).toLowerCase()}
                    {i.playTimeSeconds > 0 && ` · ${formatDuration(i.playTimeSeconds)}`}
                  </span>
                </span>
              </button>
              <PlayButton instance={i} size="sm" showProgress={false} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
