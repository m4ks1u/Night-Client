import { useEffect, useId, useState } from "react";
import { api, toCmdError, type CmdError, type LoaderKind, type LoaderVersion } from "../lib/ipc";
import { LOADER_NAME } from "../lib/keys";
import { InlineError, Skeleton, inputClass } from "./ui";

/** Segmented control for Vanilla / Fabric / NeoForge (radio group semantics). */
export function LoaderSegments({ value, onChange }: { value: LoaderKind; onChange: (v: LoaderKind) => void }) {
  const kinds: LoaderKind[] = ["vanilla", "fabric", "neoforge"];
  return (
    <div role="radiogroup" aria-label="Mod loader" className="inline-flex self-start rounded-md border border-line bg-surface-2 p-0.5">
      {kinds.map((k) => (
        <button
          key={k}
          type="button"
          role="radio"
          aria-checked={value === k}
          onClick={() => onChange(k)}
          onKeyDown={(e) => {
            const i = kinds.indexOf(value);
            if (e.key === "ArrowRight") onChange(kinds[(i + 1) % kinds.length]!);
            if (e.key === "ArrowLeft") onChange(kinds[(i + kinds.length - 1) % kinds.length]!);
          }}
          tabIndex={value === k ? 0 : -1}
          className={`h-8 rounded-[7px] px-3.5 text-[13px] font-medium transition-colors duration-(--duration-fast) ${
            value === k ? "bg-surface-3 text-fg shadow-sm" : "text-fg-muted hover:text-fg"
          }`}
        >
          {LOADER_NAME[k]}
        </button>
      ))}
    </div>
  );
}

/**
 * Loader version select for a Minecraft version. Picks the newest stable
 * version by default and reports `null` while nothing valid is selected.
 */
export function LoaderVersionSelect({ loader, minecraft, value, onChange, id }: {
  loader: Exclude<LoaderKind, "vanilla">;
  minecraft: string;
  value: string | null;
  onChange: (v: string | null) => void;
  id?: string;
}) {
  const [list, setList] = useState<LoaderVersion[] | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [showUnstable, setShowUnstable] = useState(false);
  const unstableId = useId();

  useEffect(() => {
    let alive = true;
    setList(null);
    setError(null);
    api
      .loaderVersions(loader, minecraft)
      .then((l) => {
        if (!alive) return;
        setList(l);
        const pick = l.find((v) => v.stable) ?? l[0];
        if (!value || !l.some((v) => v.version === value)) onChange(pick?.version ?? null);
      })
      .catch((e) => alive && setError(toCmdError(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loader, minecraft]);

  if (error) return <InlineError error={error} />;
  if (!list) return <Skeleton className="h-9" />;
  if (list.length === 0) {
    return (
      <p className="rounded-md border border-line bg-surface-2 px-3 py-2 text-xs text-fg-muted">
        {LOADER_NAME[loader]} isn't available for Minecraft {minecraft}. Choose another version or loader.
      </p>
    );
  }
  const shown = showUnstable ? list : list.filter((v) => v.stable || v.version === value);
  return (
    <div className="flex flex-col gap-1.5">
      <select id={id} className={inputClass} value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
        {shown.map((v, i) => (
          <option key={v.version} value={v.version}>
            {v.version}
            {i === 0 && v.stable ? " · latest" : ""}
            {!v.stable ? " · beta" : ""}
          </option>
        ))}
      </select>
      {list.some((v) => !v.stable) && (
        <label htmlFor={unstableId} className="flex items-center gap-2 text-xs text-fg-faint">
          <input id={unstableId} type="checkbox" className="accent-accent" checked={showUnstable} onChange={(e) => setShowUnstable(e.target.checked)} />
          Show beta versions
        </label>
      )}
    </div>
  );
}
