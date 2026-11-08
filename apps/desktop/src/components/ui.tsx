// Small shared primitives. All colours and timings come from design tokens.

import { useEffect, useId, useRef, type ReactNode } from "react";
import { AlertTriangle, RefreshCw } from "lucide-react";
import { Button } from "./Button";
import type { CmdError } from "../lib/ipc";
import { formatMemory } from "../lib/format";

export function PageHeader({ title, subtitle, actions }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex items-end justify-between gap-4 pb-6">
      <div className="min-w-0">
        <h1 className="truncate text-[22px] font-semibold tracking-tight">{title}</h1>
        {subtitle && <p className="mt-1 text-[13px] text-fg-muted">{subtitle}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}

export function Card({ title, children, className = "", actions }: { title?: string; children: ReactNode; className?: string; actions?: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={title ? id : undefined} className={`rounded-lg border border-line bg-surface-1 p-5 ${className}`}>
      {title && (
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 id={id} className="text-[13px] font-semibold text-fg">
            {title}
          </h2>
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

export function InlineError({ error, icon }: { error: CmdError | string; icon?: ReactNode }) {
  const message = typeof error === "string" ? error : error.message;
  return (
    <p role="alert" className="flex items-start gap-2 rounded-md bg-danger-soft px-3 py-2 text-xs leading-relaxed text-danger">
      <span className="mt-0.5 shrink-0">{icon ?? <AlertTriangle size={13} />}</span>
      <span className="select-text">{message}</span>
    </p>
  );
}

export function EmptyState({ icon, title, body, action }: { icon: ReactNode; title: string; body: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-line-strong px-6 py-14 text-center">
      <div className="mb-4 grid size-11 place-items-center rounded-full bg-accent-soft text-accent">{icon}</div>
      <h2 className="text-[15px] font-semibold">{title}</h2>
      <p className="mt-1 max-w-sm text-[13px] text-fg-muted">{body}</p>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function ErrorState({ title, error, onRetry }: { title: string; error: CmdError; onRetry?: () => void }) {
  return (
    <div role="alert" className="flex flex-col items-center justify-center rounded-lg border border-danger/25 bg-danger-soft/40 px-6 py-12 text-center">
      <AlertTriangle className="mb-3 text-danger" size={22} />
      <h2 className="text-[15px] font-semibold">{title}</h2>
      <p className="mt-1 max-w-md text-[13px] text-fg-muted select-text">{error.message}</p>
      {onRetry && (
        <Button className="mt-5" icon={<RefreshCw size={14} />} onClick={onRetry}>
          Try again
        </Button>
      )}
    </div>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div aria-hidden="true" className={`animate-pulse rounded-md bg-surface-3 ${className}`} />;
}

export function Badge({ children, tone = "neutral" }: { children: ReactNode; tone?: "neutral" | "accent" | "success" | "warning" }) {
  const tones = {
    neutral: "border-line text-fg-muted",
    accent: "border-accent/30 bg-accent-soft text-accent",
    success: "border-success/30 text-success",
    warning: "border-warning/30 text-warning",
  };
  return <span className={`inline-flex h-5 items-center rounded-sm border px-1.5 text-2xs font-medium ${tones[tone]}`}>{children}</span>;
}

export function Field({ label, hint, children, htmlFor }: { label: string; hint?: ReactNode; children: ReactNode; htmlFor: string }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-[13px] font-medium text-fg">
        {label}
      </label>
      {children}
      {hint && <p className="text-xs text-fg-faint">{hint}</p>}
    </div>
  );
}

export const inputClass =
  "h-9 w-full rounded-md border border-line bg-surface-2 px-3 text-sm text-fg placeholder:text-fg-faint transition-colors duration-(--duration-fast) hover:border-line-strong focus:border-accent focus:outline-none disabled:opacity-50";

export function Toggle({ id, checked, onChange, label, description, disabled }: {
  id: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-6">
      <div>
        <label htmlFor={id} className="text-[13px] font-medium">
          {label}
        </label>
        {description && <p className="mt-0.5 text-xs text-fg-faint">{description}</p>}
      </div>
      <button
        id={id}
        role="switch"
        type="button"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative mt-0.5 h-[20px] w-[36px] shrink-0 rounded-full transition-colors duration-(--duration-fast) disabled:opacity-50 ${checked ? "bg-accent" : "bg-surface-3 border border-line-strong"}`}
      >
        <span
          className={`absolute top-1/2 size-[14px] -translate-y-1/2 rounded-full bg-fg transition-[left] duration-(--duration-fast) ease-(--ease-out) ${checked ? "left-[18px]" : "left-[3px]"}`}
        />
      </button>
    </div>
  );
}

export function MemorySlider({ value, onChange, max, recommended, disabled, label = "Memory" }: {
  value: number;
  onChange: (v: number) => void;
  max: number;
  recommended?: number;
  disabled?: boolean;
  label?: string;
}) {
  const id = useId();
  const tooHigh = recommended !== undefined && value > recommended * 2;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-[13px] font-medium">
          {label}
        </label>
        <span className="text-[13px] font-medium tabular-nums">{formatMemory(value)}</span>
      </div>
      <input
        id={id}
        type="range"
        min={1024}
        max={Math.max(1024, max)}
        step={512}
        value={Math.min(value, Math.max(1024, max))}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-valuetext={formatMemory(value)}
        className="w-full accent-accent disabled:opacity-50"
      />
      <p className={`text-xs ${tooHigh ? "text-warning" : "text-fg-faint"}`}>
        {tooHigh
          ? "More memory than needed can cause longer garbage-collection pauses."
          : recommended
            ? `Recommended for this PC: ${formatMemory(recommended)}`
            : null}
      </p>
    </div>
  );
}

/** Accessible modal built on <dialog>: focus trap, Escape and inert background for free. */
export function Dialog({ open, onClose, title, children, footer }: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  footer: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      aria-labelledby={id}
      onClose={onClose}
      onClick={(e) => e.target === ref.current && onClose()}
      className="m-auto w-[420px] max-w-[calc(100vw-32px)] rounded-lg border border-line-strong bg-surface-2 p-0 text-fg shadow-2xl backdrop:bg-black/60"
    >
      <div className="p-5">
        <h2 id={id} className="text-[15px] font-semibold">
          {title}
        </h2>
        <div className="mt-2 text-[13px] text-fg-muted">{children}</div>
      </div>
      <div className="flex justify-end gap-2 border-t border-line px-5 py-3">{footer}</div>
    </dialog>
  );
}
