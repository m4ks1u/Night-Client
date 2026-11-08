import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Download, ExternalLink, FileUp, Heart, Search } from "lucide-react";
import { Button } from "./Button";
import { ModIcon } from "./ModParts";
import { ProgressBar } from "./ProgressBar";
import { Badge, Dialog, ErrorState, Field, InlineError, PageHeader, Skeleton, inputClass } from "./ui";
import { api, events, toCmdError, type CmdError, type ModpackHit, type ModpackProgress, type PackPreview, type SortIndex } from "../lib/ipc";
import { COMPAT_TAGS } from "../lib/content";
import { formatBytes, formatCount } from "../lib/format";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

const SORTS: { value: SortIndex; label: string }[] = [
  { value: "relevance", label: "Relevance" },
  { value: "downloads", label: "Downloads" },
  { value: "follows", label: "Followers" },
  { value: "updated", label: "Recently updated" },
  { value: "newest", label: "Newest" },
];

/** Discover → Modpacks: each pack becomes a new instance. */
export function ModpackDiscover({ switcher }: { switcher: ReactNode }) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<SortIndex>("relevance");
  const [hits, setHits] = useState<ModpackHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<CmdError | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const ids = { sort: useId(), q: useId() };
  const request = useRef(0);
  const importer = useModpackImporter();

  useEffect(() => {
    const t = setTimeout(() => setDebounced(query), 300);
    return () => clearTimeout(t);
  }, [query]);

  const search = useCallback(
    async (offset: number) => {
      const mine = ++request.current;
      if (offset === 0) setHits(null);
      setError(null);
      try {
        const r = await api.modpackSearch(debounced, sort, offset);
        if (mine !== request.current) return;
        setTotal(r.total);
        setHits((prev) => (offset === 0 || !prev ? r.hits : [...prev, ...r.hits]));
      } catch (e) {
        if (mine === request.current) setError(toCmdError(e));
      }
    },
    [debounced, sort],
  );
  useEffect(() => void search(0), [search]);

  return (
    <div>
      <PageHeader
        title="Discover"
        subtitle="Modpacks from Modrinth. Each one is installed as a new instance."
        actions={
          <Button icon={<FileUp size={15} />} onClick={() => void importer.fromFile()}>
            Import .mrpack file
          </Button>
        }
      />
      {switcher}

      <div className="mb-4 flex gap-2">
        <div className="relative min-w-0 flex-1">
          <Search size={14} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-fg-faint" />
          <label htmlFor={ids.q} className="sr-only">
            Search modpacks
          </label>
          <input id={ids.q} type="search" placeholder="Search modpacks" value={query} onChange={(e) => setQuery(e.target.value)} className={`${inputClass} pl-8`} />
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

      {error ? (
        <ErrorState title="Modrinth could not be searched" error={error} onRetry={() => void search(0)} />
      ) : !hits ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-20" />
          ))}
        </div>
      ) : hits.length === 0 ? (
        <p className="py-12 text-center text-[13px] text-fg-muted">No modpacks match. Try other words.</p>
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
                      .filter((c) => !COMPAT_TAGS.includes(c))
                      .slice(0, 3)
                      .map((c) => (
                        <Badge key={c}>{c.replace(/-/g, " ")}</Badge>
                      ))}
                  </p>
                </div>
                <div className="flex shrink-0 flex-col items-end justify-between gap-2">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={importer.loading === h.project_id}
                    onClick={() => void importer.fromModrinth(h.project_id)}
                    aria-label={`Install ${h.title}`}
                  >
                    {importer.loading === h.project_id ? "Reading…" : "Install"}
                  </Button>
                  {h.slug && (
                    <button
                      type="button"
                      onClick={() => void api.openModpackPage(h.slug!)}
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
      {importer.dialog}
    </div>
  );
}

/** Preview a pack (from a file or Modrinth) and import it from a dialog. */
export function useModpackImporter() {
  const [preview, setPreview] = useState<PackPreview | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const [error, setError] = useState<CmdError | null>(null);

  async function run(key: string, f: () => Promise<PackPreview | null>) {
    setLoading(key);
    setError(null);
    try {
      const p = await f();
      if (p) setPreview(p);
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setLoading(null);
    }
  }

  return {
    loading,
    fromFile: () => run("file", api.modpackPreviewFile),
    fromModrinth: (projectId: string) => run(projectId, () => api.modpackPreviewModrinth(projectId)),
    dialog: (
      <>
        <Dialog open={error !== null} onClose={() => setError(null)} title="This modpack can't be installed" footer={<Button onClick={() => setError(null)}>Close</Button>}>
          {error && <InlineError error={error} />}
        </Dialog>
        <ModpackImportDialog preview={preview} onClose={() => setPreview(null)} />
      </>
    ),
  };
}

function ModpackImportDialog({ preview, onClose }: { preview: PackPreview | null; onClose: () => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ModpackProgress | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const upsert = useApp((s) => s.upsertInstance);
  const installVersion = useApp((s) => s.installVersion);
  const nameId = useId();

  useEffect(() => {
    setName(preview?.name.slice(0, 64) ?? "");
    setProgress(null);
    setError(null);
  }, [preview]);
  useEffect(() => {
    if (!preview) return;
    const un = events.modpackProgress((p) => p.token === preview.token && setProgress(p));
    return () => void un.then((f) => f());
  }, [preview]);

  if (!preview) return <Dialog open={false} onClose={onClose} title="" footer={null}>{null}</Dialog>;
  const trimmed = name.trim();
  const nameBad = trimmed.length === 0 || trimmed.length > 64;

  async function start() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const inst = await api.modpackImport(preview.token, trimmed);
      upsert(inst);
      onClose();
      navigate({ name: "instance", id: inst.id, tab: inst.loader === "vanilla" ? "overview" : "mods" });
      if (!inst.installed && inst.installKey) void installVersion(inst.installKey);
    } catch (e) {
      const err = toCmdError(e);
      // The token is used up either way: a retry needs a fresh preview.
      setError(err.code === "cancelled" ? null : err);
      if (err.code === "cancelled") onClose();
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  const pct = progress ? (progress.totalBytes > 0 ? progress.doneBytes / progress.totalBytes : progress.totalFiles > 0 ? progress.doneFiles / progress.totalFiles : null) : null;
  const usedUp = error !== null;
  return (
    <Dialog
      open
      onClose={() => (busy ? void api.modpackCancel(preview.token) : onClose())}
      title={`Install ${preview.name}`}
      footer={
        busy ? (
          <Button onClick={() => void api.modpackCancel(preview.token)}>Cancel</Button>
        ) : (
          <>
            <Button onClick={onClose}>{usedUp ? "Close" : "Cancel"}</Button>
            {!usedUp && (
              <Button variant="primary" disabled={nameBad} onClick={() => void start()}>
                Create instance
              </Button>
            )}
          </>
        )
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex gap-3">
          <ModIcon url={preview.iconUrl} size={40} />
          <div className="min-w-0">
            <p className="text-fg">{preview.label}</p>
            <p className="text-xs">
              Version {preview.versionId} · {preview.files} files · {formatBytes(preview.downloadBytes)} to download
            </p>
          </div>
        </div>
        {preview.summary && <p className="line-clamp-3 text-xs">{preview.summary}</p>}
        {preview.skipped > 0 && <p className="text-xs">{preview.skipped} server-only files are left out.</p>}
        {preview.optionalFiles > 0 && <p className="text-xs">{preview.optionalFiles} optional files are included; you can disable them in the Mods tab.</p>}
        <Field label="Instance name" htmlFor={nameId}>
          <input id={nameId} className={inputClass} value={name} maxLength={64} disabled={busy || usedUp} onChange={(e) => setName(e.target.value)} />
        </Field>
        {busy && (
          <div className="flex flex-col gap-1.5" aria-live="polite">
            <ProgressBar value={pct} label="Modpack download" />
            <p className="text-xs">
              {!progress
                ? "Creating the instance…"
                : progress.phase === "overrides"
                  ? "Copying configuration files…"
                  : `${progress.doneFiles} of ${progress.totalFiles} files · ${formatBytes(progress.doneBytes)} of ${formatBytes(progress.totalBytes)}`}
            </p>
          </div>
        )}
        {error && <InlineError error={error} />}
        <p className="text-xs text-fg-faint">Files come from the hosts the pack lists (Modrinth, GitHub, GitLab) and are checked against the pack's hashes.</p>
      </div>
    </Dialog>
  );
}
