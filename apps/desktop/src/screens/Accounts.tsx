import { useCallback, useEffect, useState } from "react";
import { Check, Loader2, LogIn, Plus, ShieldCheck, Trash2 } from "lucide-react";
import { Button } from "../components/Button";
import { Badge, Card, Dialog, ErrorState, InlineError, PageHeader, Skeleton } from "../components/ui";
import { api, toCmdError, type Account, type CmdError } from "../lib/ipc";
import { formatRelative } from "../lib/format";
import { useApp } from "../stores/app";

export function Accounts() {
  const info = useApp((s) => s.info);
  const setActive = useApp((s) => s.setAccount);
  const running = useApp((s) => Object.keys(s.running).length > 0);
  const [list, setList] = useState<Account[] | null>(null);
  const [loadError, setLoadError] = useState<CmdError | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CmdError | null>(null);
  const [removing, setRemoving] = useState<Account | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const l = await api.listAccounts();
      setList(l);
      setActive(l.find((a) => a.active) ?? null);
    } catch (e) {
      setLoadError(toCmdError(e));
    }
  }, [setActive]);
  useEffect(() => void load(), [load]);

  async function signIn() {
    setBusy(true);
    setError(null);
    try {
      await api.signIn();
      await load();
    } catch (e) {
      const err = toCmdError(e);
      if (err.code !== "cancelled") setError(err);
    } finally {
      setBusy(false);
    }
  }

  async function act(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(toCmdError(e));
    }
  }

  const configured = info?.signInConfigured ?? false;

  return (
    <div className="max-w-2xl">
      <PageHeader
        title="Accounts"
        subtitle="Sign in with the Microsoft accounts that own Minecraft: Java Edition. You stay signed in between restarts."
        actions={
          list && list.length > 0 && !busy ? (
            <Button icon={<Plus size={14} />} disabled={!configured} onClick={() => void signIn()}>
              Add account
            </Button>
          ) : null
        }
      />

      <div className="flex flex-col gap-4">
        {busy && (
          <Card>
            <div className="flex items-center justify-between gap-4">
              <p className="flex items-center gap-2 text-[13px] text-fg-muted">
                <Loader2 size={15} className="animate-spin" /> Finish signing in in your browser…
              </p>
              <Button onClick={() => void api.cancelSignIn()}>Cancel</Button>
            </div>
          </Card>
        )}
        {error && <InlineError error={error} />}

        {loadError ? (
          <ErrorState title="Accounts could not be loaded" error={loadError} onRetry={() => void load()} />
        ) : !list ? (
          <Skeleton className="h-20" />
        ) : list.length === 0 ? (
          !busy && (
            <Card>
              <div className="flex flex-col items-start gap-3">
                <Button variant="primary" icon={<LogIn size={15} />} disabled={!configured} onClick={() => void signIn()}>
                  Sign in with Microsoft
                </Button>
                {!configured && (
                  <p className="text-xs leading-relaxed text-fg-faint">
                    This build has no approved Microsoft application ID, so sign-in is unavailable. Builders: set{" "}
                    <code className="font-mono">NIGHT_CLIENT_MSA_CLIENT_ID</code> and restart.
                  </p>
                )}
              </div>
            </Card>
          )
        ) : (
          <ul className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface-1">
            {list.map((a) => (
              <li key={a.uuid} className="flex items-center gap-3 px-4 py-3.5">
                <span
                  className={`grid size-9 shrink-0 place-items-center rounded-full text-[14px] font-semibold ${a.active ? "bg-accent-soft text-accent" : "bg-surface-3 text-fg-muted"}`}
                  aria-hidden="true"
                >
                  {a.name.slice(0, 1).toUpperCase()}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2 text-[13px] font-medium">
                    <span className="truncate">{a.name}</span>
                    {a.active && <Badge tone="accent">Active</Badge>}
                    {a.status === "expired" && <Badge tone="warning">Sign-in expired</Badge>}
                  </p>
                  <p className="text-xs text-fg-faint">Last used {formatRelative(a.lastUsedAt).toLowerCase()}</p>
                </div>
                {a.status === "expired" ? (
                  <Button size="sm" icon={<LogIn size={13} />} disabled={!configured || busy} onClick={() => void signIn()} title="Choose this account again in the browser">
                    Sign in again
                  </Button>
                ) : (
                  !a.active && (
                    <Button size="sm" icon={<Check size={13} />} disabled={running} title={running ? "Close running games first" : undefined} onClick={() => void act(() => api.switchAccount(a.uuid))}>
                      Use
                    </Button>
                  )
                )}
                <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} aria-label={`Remove ${a.name}`} onClick={() => setRemoving(a)} />
              </li>
            ))}
          </ul>
        )}

        <Card>
          <div className="flex gap-3">
            <ShieldCheck size={18} className="mt-0.5 shrink-0 text-accent" />
            <div className="text-[13px] leading-relaxed text-fg-muted">
              <p>
                Sign-in happens in your browser on Microsoft's website, so Night Client never sees your password. To keep you
                signed in, a renewal token is encrypted with Windows (DPAPI) for your Windows user on this PC only. It is never
                written to the database, logs or the screen.
              </p>
            </div>
          </div>
        </Card>
      </div>

      <Dialog
        open={removing !== null}
        onClose={() => setRemoving(null)}
        title={`Remove ${removing?.name ?? ""}?`}
        footer={
          <>
            <Button onClick={() => setRemoving(null)}>Cancel</Button>
            <Button
              variant="danger"
              onClick={() => {
                const a = removing;
                setRemoving(null);
                if (a) void act(() => api.removeAccount(a.uuid));
              }}
            >
              Remove
            </Button>
          </>
        }
      >
        Night Client deletes its saved sign-in for this account. Your Microsoft account, worlds and instances are not affected.
      </Dialog>
    </div>
  );
}
