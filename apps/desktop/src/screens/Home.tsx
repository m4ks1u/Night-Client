import { Boxes, Clock, Plus } from "lucide-react";
import { Button } from "../components/Button";
import { PlayButton } from "../components/InstanceActions";
import { Badge, EmptyState, ErrorState, PageHeader, Skeleton } from "../components/ui";
import { formatDuration, formatRelative } from "../lib/format";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

function greeting(d = new Date()) {
  const h = d.getHours();
  return h < 5 ? "Good night" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
}

export function Home() {
  const account = useApp((s) => s.account);
  const instances = useApp((s) => s.instances);
  const refresh = useApp((s) => s.refreshInstances);

  return (
    <div>
      <PageHeader title={account ? `${greeting()}, ${account.name}` : greeting()} subtitle="Pick up where you left off." />

      {instances.status === "error" ? (
        <ErrorState title="Instances could not be loaded" error={instances.error} onRetry={() => void refresh()} />
      ) : instances.status !== "ready" ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-40" />
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
        </div>
      ) : instances.data.length === 0 ? (
        <EmptyState
          icon={<Boxes size={20} />}
          title="Create your first instance"
          body="An instance is a separate Minecraft installation with its own worlds, settings and, later, mods."
          action={
            <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })}>
              New instance
            </Button>
          }
        />
      ) : (
        (() => {
          const [first, ...rest] = instances.data;
          if (!first) return null;
          return (
            <div className="flex flex-col gap-8">
              <section aria-label="Continue playing" className="relative overflow-hidden rounded-lg border border-line bg-surface-1">
                <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(120%_90%_at_100%_0%,var(--color-accent-soft),transparent_60%)]" aria-hidden="true" />
                <div className="relative flex items-end justify-between gap-6 p-6">
                  <div className="min-w-0">
                    <p className="text-2xs font-semibold tracking-[0.08em] text-fg-faint uppercase">
                      {first.lastPlayedAt ? "Continue playing" : "Ready to play"}
                    </p>
                    <button
                      type="button"
                      onClick={() => navigate({ name: "instance", id: first.id })}
                      className="mt-2 block max-w-full truncate text-left text-[26px] font-semibold tracking-tight hover:text-accent-hover"
                    >
                      {first.name}
                    </button>
                    <div className="mt-2 flex items-center gap-3 text-xs text-fg-muted">
                      <Badge>Vanilla {first.gameVersion}</Badge>
                      <span className="flex items-center gap-1">
                        <Clock size={12} /> {formatRelative(first.lastPlayedAt)}
                      </span>
                      {first.playTimeSeconds > 0 && <span>{formatDuration(first.playTimeSeconds)} played</span>}
                    </div>
                  </div>
                  <PlayButton instance={first} size="lg" />
                </div>
              </section>

              {rest.length > 0 && (
                <section aria-labelledby="recent">
                  <div className="mb-3 flex items-center justify-between">
                    <h2 id="recent" className="text-[13px] font-semibold">
                      Other instances
                    </h2>
                    <Button variant="ghost" size="sm" onClick={() => navigate({ name: "instances" })}>
                      View all
                    </Button>
                  </div>
                  <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
                    {rest.slice(0, 5).map((i) => (
                      <li key={i.id} className="flex items-center justify-between gap-4 px-4 py-3">
                        <button type="button" className="min-w-0 text-left" onClick={() => navigate({ name: "instance", id: i.id })}>
                          <span className="block truncate text-[13px] font-medium hover:text-accent-hover">{i.name}</span>
                          <span className="text-xs text-fg-faint">
                            {i.gameVersion} · {formatRelative(i.lastPlayedAt)}
                          </span>
                        </button>
                        <PlayButton instance={i} size="sm" showProgress={false} />
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </div>
          );
        })()
      )}
    </div>
  );
}
