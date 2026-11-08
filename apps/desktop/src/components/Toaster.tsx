import { useEffect, useRef } from "react";
import { AnimatePresence, m } from "motion/react";
import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import { useToasts, type Toast } from "../stores/toast";
import { navigate } from "../stores/router";

const ICON = {
  success: <CheckCircle2 size={16} className="text-success" />,
  error: <AlertTriangle size={16} className="text-danger" />,
  info: <Info size={16} className="text-info" />,
};

/** Bottom-right notifications for background results. Polite live region. */
export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  return (
    <section aria-label="Notifications" aria-live="polite" className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-[340px] flex-col gap-2">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <m.div
            key={t.id}
            layout
            initial={{ opacity: 0, y: 8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, x: 16 }}
            transition={{ duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <ToastCard t={t} />
          </m.div>
        ))}
      </AnimatePresence>
    </section>
  );
}

function ToastCard({ t }: { t: Toast }) {
  const dismiss = useToasts((s) => s.dismiss);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const start = () => {
    clearTimeout(timer.current);
    // Errors stay longer: they usually need reading.
    timer.current = setTimeout(() => dismiss(t.id), t.tone === "error" ? 12000 : 6000);
  };
  useEffect(() => {
    start();
    return () => clearTimeout(timer.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div
      role={t.tone === "error" ? "alert" : "status"}
      onMouseEnter={() => clearTimeout(timer.current)}
      onMouseLeave={start}
      onFocus={() => clearTimeout(timer.current)}
      onBlur={start}
      className="pointer-events-auto flex items-start gap-3 rounded-lg border border-line-strong bg-surface-2 p-3.5 shadow-2xl"
    >
      <span className="mt-0.5 shrink-0">{ICON[t.tone]}</span>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-medium">{t.title}</p>
        {t.body && <p className="mt-0.5 text-xs text-fg-muted select-text">{t.body}</p>}
        {t.action && (
          <button
            type="button"
            className="mt-1.5 text-xs font-medium text-accent hover:text-accent-hover"
            onClick={() => {
              navigate(t.action!.route);
              dismiss(t.id);
            }}
          >
            {t.action.label}
          </button>
        )}
      </div>
      <button type="button" aria-label="Dismiss notification" onClick={() => dismiss(t.id)} className="shrink-0 rounded p-0.5 text-fg-faint hover:text-fg">
        <X size={14} />
      </button>
    </div>
  );
}
