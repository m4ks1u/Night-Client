import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Check, Compass, Download, ExternalLink, Heart, Plus, Search } from "lucide-react";
import { Button } from "../components/Button";
import { InstallPlanDialog, ModIcon } from "../components/ModParts";
import { Badge, EmptyState, ErrorState, PageHeader, Skeleton, inputClass } from "../components/ui";
import { api, toCmdError, type Category, type CmdError, type SearchHit, type SortIndex } from "../lib/ipc";
import { formatCount } from "../lib/format";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

const SORTS: { value: SortIndex; label: string }[] = [
  { value: "relevance", label: "Relevance" },
  { value: "downloads", label: "Downloads" },
  { value: "follows", label: "Followers" },
  { value: "updated", label: "Recently updated" },
  { value: "newest", label: "Newest" },
];

export function Discover({ instanceId }: { instanceId?: string }) {
  const instances = useApp((s) => s.instances);
  const modded = useMemo(() => (instances.status === "ready" ? instances.data.filter((i) => i.loader !== "vanilla") : []), [instances]);
  const [selected, setSelected] = useState<string | null>(instanceId ?? null);
  const instance = modded.find((i) => i.id === selected) ?? modded[0] ?? null;

  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<SortIndex>("relevance");
  const [category, setCategory] = useState<string | null>(null);
  const [categories, setCategories] = useState<Category[]>([]);
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<CmdError | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [installing, setInstalling] = useState<SearchHit | null>(null);
  const ids = { inst: useId(), sort: useId(), q: useId() };
  const request = useRef(0);

  useEffect(() => {
    const t = setTimeout(() => setDebounced(query), 300);
    return () => clearTimeout(t);
  }, [query]);
  useEffect(() => {
    api.modrinthCategories().then(setCategories).catch(() => setCategories([]));
  }, []);

  const search = useCallback(
    async (offset: number) => {
      if (!instance) return;
      const mine = ++request.current;
      if (offset === 0) setHits(null);
      setError(null);
      try {
        const r = await api.modrinthSearch(instance.id, debounced, category, sort, offset);
        if (mine !== request.current) return; // a newer search started
        setTotal(r.total);
        setHits((prev) => (offset === 0 || !prev ? r.hits : [...prev, ...r.hits]));
      } catch (e) {
        if (mine === request.current) setError(toCmdError(e));
      }
    },
    [instance, debounced, category, sort],
  );
  useEffect(() => void search(0), [search]);

  if (instances.status === "ready" && modded.length === 0) {
    return (
      <div>
        <PageHeader title="Discover mods" subtitle="Browse Modrinth for Fabric, Quilt, NeoForge and Forge mods." />
        <EmptyState
          icon={<Compass size={20} />}
          title="Create a modded instance first"
          body="Mods need a mod loader. Create an instance with Fabric, Quilt, NeoForge or Forge, then come back here."
          action={
            <Button variant="primary" icon={<Plus size={15} />} onClick={() => navigate({ name: "new-instance" })}>
              New instance
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Discover mods"
        subtitle={instance ? `Showing mods for ${instance.label}. Results from Modrinth.` : "Results from Modrinth."}
        actions={
          modded.length > 0 && (
            <>
              <label htmlFor={ids.inst} className="sr-only">
                Instance
              </label>
              <div className="w-64">
              <select id={ids.inst} className={inputClass} value={instance?.id ?? ""} onChange={(e) => setSelected(e.target.value)}>
                {modded.map((i) => (
                  <option key={i.id} value={i.id}>
                    {i.name} · {i.label}
                  </option>
                ))}
              </select>
              </div>
            </>
          )
        }
      />

      <div className="mb-3 flex gap-2">
        <div className="relative min-w-0 flex-1">
          <Search size={14} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-fg-faint" />
          <label htmlFor={ids.q} className="sr-only">
            Search mods
          </label>
          <input id={ids.q} type="search" placeholder="Search mods" value={query} onChange={(e) => setQuery(e.target.value)} className={`${inputClass} pl-8`} />
        </div>
        <label htmlFor={ids.sort} className="sr-only">
          Sort by
        </label>
        <div className="w-44 shrink-0">
        <select id={ids.sort} className={inputClass} value={sort} onChange={(e) => setSort(e.target.value as SortIndex)}>
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        </div>
      </div>

      {categories.length > 0 && (
        <div role="group" aria-label="Categories" className="mb-5 flex flex-wrap gap-1.5">
          {[null, ...categories.map((c) => c.name)].map((c) => (
            <button
              key={c ?? "all"}
              type="button"
              aria-pressed={category === c}
              onClick={() => setCategory(c)}
              className={`h-7 rounded-full border px-3 text-xs capitalize transition-colors duration-(--duration-fast) ${
                category === c ? "border-accent/50 bg-accent-soft text-accent" : "border-line text-fg-muted hover:border-line-strong hover:text-fg"
              }`}
            >
              {c ? c.replace(/-/g, " ") : "All"}
            </button>
          ))}
        </div>
      )}

      {error ? (
        <ErrorState title="Modrinth could not be searched" error={error} onRetry={() => void search(0)} />
      ) : !hits ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-20" />
          ))}
        </div>
      ) : hits.length === 0 ? (
        <p className="py-12 text-center text-[13px] text-fg-muted">No mods match. Try other words or another category.</p>
      ) : (
        <>
          <p className="mb-2 text-xs text-fg-faint" aria-live="polite">
            {total.toLocaleString()} results
          </p>
          <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
            {hits.map((h) => (
              <li key={h.project_id} className="flex gap-4 px-4 py-3.5">
                <ModIcon url={h.icon_url} size={48} />
                <div className="min-w-0 flex-1">
                  <p className="flex items-baseline gap-2">
                    <span className="truncate text-[14px] font-semibold">{h.title}</span>
                    <span className="truncate text-xs text-fg-faint">by {h.author}</span>
                  </p>
                  <p className="mt-0.5 line-clamp-2 text-[13px] text-fg-muted">{h.description}</p>
                  <p className="mt-1.5 flex flex-wrap items-center gap-3 text-xs text-fg-faint">
                    <span className="flex items-center gap-1"><Download size={12} /> {formatCount(h.downloads)}</span>
                    <span className="flex items-center gap-1"><Heart size={12} /> {formatCount(h.follows)}</span>
                    {h.display_categories
                      .filter((c) => !["fabric", "neoforge", "forge", "quilt"].includes(c))
                      .slice(0, 3)
                      .map((c) => (
                        <Badge key={c}>{c.replace(/-/g, " ")}</Badge>
                      ))}
                  </p>
                </div>
                <div className="flex shrink-0 flex-col items-end justify-between gap-2">
                  {h.installed ? (
                    <Badge tone="success">
                      <Check size={11} className="mr-1" /> Installed
                    </Badge>
                  ) : (
                    <Button size="sm" variant="primary" disabled={!instance || instance.id in useApp.getState().running} onClick={() => setInstalling(h)}>
                      Install
                    </Button>
                  )}
                  {h.slug && (
                    <button
                      type="button"
                      onClick={() => void api.openModrinthPage(h.slug!)}
                      className="flex items-center gap-1 text-xs text-fg-faint hover:text-fg"
                      aria-label={`Open ${h.title} on Modrinth`}
                    >
                      Modrinth <ExternalLink size={11} />
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
          {hits.length < total && (
            <div className="mt-4 flex justify-center">
              <Button
                disabled={loadingMore}
                onClick={async () => {
                  setLoadingMore(true);
                  await search(hits.length);
                  setLoadingMore(false);
                }}
              >
                {loadingMore ? "Loading…" : "Load more"}
              </Button>
            </div>
          )}
        </>
      )}

      {instance && (
        <InstallPlanDialog
          instanceId={instance.id}
          projectId={installing?.project_id ?? null}
          title={installing?.title ?? ""}
          onClose={() => setInstalling(null)}
          onInstalled={() => {
            const done = installing?.project_id;
            setHits((prev) => prev?.map((h) => (h.project_id === done ? { ...h, installed: true } : h)) ?? prev);
            void search(0);
          }}
        />
      )}
    </div>
  );
}
