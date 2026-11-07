type Props = {
  /** 0..1, or `null` for indeterminate. */
  value: number | null;
  label: string;
};

export function ProgressBar({ value, label }: Props) {
  const pct = value === null ? undefined : Math.round(Math.max(0, Math.min(1, value)) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      className="relative h-1.5 w-full overflow-hidden rounded-full bg-surface-3"
    >
      {value === null ? (
        <div className="animate-indeterminate absolute inset-y-0 w-1/3 rounded-full bg-accent/70" />
      ) : (
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-(--duration-base) ease-(--ease-out)"
          style={{ width: `${pct}%` }}
        />
      )}
    </div>
  );
}
