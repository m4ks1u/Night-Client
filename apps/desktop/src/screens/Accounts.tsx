import { useState } from "react";
import { Loader2, LogIn, LogOut, ShieldCheck } from "lucide-react";
import { Button } from "../components/Button";
import { Card, InlineError, PageHeader } from "../components/ui";
import { api, toCmdError, type CmdError } from "../lib/ipc";
import { useApp } from "../stores/app";

export function Accounts() {
  const account = useApp((s) => s.account);
  const setAccount = useApp((s) => s.setAccount);
  const info = useApp((s) => s.info);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CmdError | null>(null);

  async function signIn() {
    setBusy(true);
    setError(null);
    try {
      setAccount(await api.signIn());
    } catch (e) {
      const err = toCmdError(e);
      if (err.code !== "cancelled") setError(err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="max-w-2xl">
      <PageHeader title="Account" subtitle="Sign in with the Microsoft account that owns Minecraft: Java Edition." />

      <div className="flex flex-col gap-4">
        <Card>
          {account ? (
            <div className="flex items-center justify-between gap-4">
              <div className="flex min-w-0 items-center gap-3">
                <span className="grid size-10 place-items-center rounded-full bg-accent-soft text-[15px] font-semibold text-accent" aria-hidden="true">
                  {account.name.slice(0, 1).toUpperCase()}
                </span>
                <div className="min-w-0">
                  <p className="truncate text-[15px] font-semibold">{account.name}</p>
                  <p className="text-xs text-fg-faint">Microsoft account · session ends when Night Client closes</p>
                </div>
              </div>
              <Button icon={<LogOut size={14} />} onClick={() => void api.signOut().then(() => setAccount(null))}>
                Sign out
              </Button>
            </div>
          ) : busy ? (
            <div className="flex items-center justify-between gap-4">
              <p className="flex items-center gap-2 text-[13px] text-fg-muted">
                <Loader2 size={15} className="animate-spin" /> Finish signing in in your browser…
              </p>
              <Button onClick={() => void api.cancelSignIn()}>Cancel</Button>
            </div>
          ) : (
            <div className="flex flex-col items-start gap-3">
              <Button variant="primary" icon={<LogIn size={15} />} disabled={!info?.signInConfigured} onClick={() => void signIn()}>
                Sign in with Microsoft
              </Button>
              {info && !info.signInConfigured && (
                <p className="text-xs leading-relaxed text-fg-faint">
                  This build has no approved Microsoft application ID, so sign-in is unavailable. Builders: set{" "}
                  <code className="font-mono">NIGHT_CLIENT_MSA_CLIENT_ID</code> and restart.
                </p>
              )}
            </div>
          )}
          {error && <div className="mt-4"><InlineError error={error} /></div>}
        </Card>

        <Card>
          <div className="flex gap-3">
            <ShieldCheck size={18} className="mt-0.5 shrink-0 text-accent" />
            <div className="text-[13px] leading-relaxed text-fg-muted">
              <p>
                Sign-in happens in your own browser on Microsoft's website. Night Client never sees your password and stores no
                tokens in its database or logs.
              </p>
              <p className="mt-2 text-xs text-fg-faint">
                Staying signed in between restarts (using Windows-protected storage) and switching between several accounts arrive
                in a later update.
              </p>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
