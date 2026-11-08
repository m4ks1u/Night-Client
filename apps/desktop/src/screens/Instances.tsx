import { useMemo, useState } from "react";
import { Boxes, Plus, Search } from "lucide-react";
import { Button } from "../components/Button";
import { PlayButton } from "../components/InstanceActions";
import { Badge, EmptyState, ErrorState, PageHeader, Skeleton, inputClass } from "../components/ui";
import { formatDuration, formatRelative } from "../lib/format";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

export function Instances() {
  const instances = useApp((s) => s.instances);
  const running = useApp((s) => s.running);
  const refresh = useApp((s) => s.refreshInstances);
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    if (instances.status !== "ready") return [];
    const q = query.trim().toLowerCase();
    return q ? instances.data.filter((i) => i.name.toLowerCase().includes(q) || i.label.toLowerCase().includes(q)) : instances.data;
  }, [instances, query]);

  const hasAny = instances.status === "ready" && instances.data.length > 0;

  return (
    <div>
      <PageHeader
        title="Instances"
        subtitle="Each instance has its own game folder, worlds and settings."
        actions={
          <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })}>
            New instance
          </Button>
        }
      />

      {hasAny && (
        <div className="relative mb-4 max-w-sm">
          <Search size={14} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-fg-faint" />
          <input
            type="search"
            aria-label="Search instances"
            placeholder="Search by name or version"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className={`${inputClass} pl-8`}
          />
        </div>
      )}

      {instances.status === "error" ? (
        <ErrorState title="Instances could not be loaded" error={instances.error} onRetry={() => void refresh()} />
      ) : instances.status !== "ready" ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((i) => <Skeleton key={i} className="h-16" />)}
        </div>
      ) : !hasAny ? (
        <EmptyState
          icon={<Boxes size={20} />}
          title="No instances yet"
          body="Create an instance to choose a Minecraft version and start playing."
          action={
            <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })}>
              New instance
            </Button>
          }
        />
      ) : filtered.length === 0 ? (
        <p className="py-10 text-center text-[13px] text-fg-muted">No instances match “{query}”.</p>
      ) : (
        <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
          {filtered.map((i) => (
            <li key={i.id} className="flex items-center gap-4 px-4 py-3.5 transition-colors hover:bg-surface-2/60">
              <button type="button" className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => navigate({ name: "instance", id: i.id })}>
                <span className="grid size-9 shrink-0 place-items-center rounded-md border border-line bg-surface-2 text-[13px] font-semibold text-fg-muted" aria-hidden="true">
                  {i.name.slice(0, 1).toUpperCase()}
                </span>
                <span className="min-w-0">
                  <span className="flex items-center gap-2">
                    <span className="truncate text-[13px] font-medium">{i.name}</span>
                    {i.id in running ? <Badge tone="success">Running</Badge> : !i.installed && <Badge tone="warning">Not installed</Badge>}
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
