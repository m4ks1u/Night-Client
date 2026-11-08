import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Boxes, FolderOpen, Moon, ShieldCheck, UserRound } from "lucide-react";
import { Button } from "./Button";
import { Toggle } from "./ui";
import { api, type Settings } from "../lib/ipc";
import { useApp } from "../stores/app";
import { navigate, type Route } from "../stores/router";
import { create } from "zustand";

/** Lets Settings reopen the welcome after it was finished. */
export const useWelcome = create<{ reopened: boolean; reopen: () => void; close: () => void }>((set) => ({
  reopened: false,
  reopen: () => set({ reopened: true }),
  close: () => set({ reopened: false }),
}));

const STEPS = ["Welcome", "Your game data", "Privacy", "Get started"] as const;

/** First-run welcome: four short steps, shown once (finish or skip). */
export function Welcome() {
  const settings = useApp((s) => s.settings);
  const info = useApp((s) => s.info);
  const reopened = useWelcome((s) => s.reopened);
  const open = settings !== null && (!settings.onboarded || reopened);
  const [step, setStep] = useState(0);
  const [checkUpdates, setCheckUpdates] = useState(true);
  const ref = useRef<HTMLDialogElement>(null);
  const nextRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const toggleId = useId();

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      setStep(0);
      setCheckUpdates(settings?.checkUpdates ?? true);
      d.showModal();
      // showModal focuses the first button (Skip); start on Next instead.
      requestAnimationFrame(() => nextRef.current?.focus());
    }
    if (!open && d.open) d.close();
  }, [open, settings?.checkUpdates]);

  async function finish(to?: Route) {
    const s = useApp.getState().settings;
    if (s) {
      const next: Settings = { ...s, onboarded: true, checkUpdates };
      try {
        useApp.getState().setSettings(await api.updateSettings(next));
      } catch {
        useApp.getState().setSettings(next); // keep the UI usable; it shows again next start
      }
    }
    useWelcome.getState().close();
    if (to) navigate(to);
  }

  const last = step === STEPS.length - 1;
  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      // Esc counts as "skip": the user has seen the welcome.
      onCancel={(e) => {
        e.preventDefault();
        void finish();
      }}
      className="m-auto w-[520px] max-w-[calc(100vw-32px)] rounded-lg border border-line-strong bg-surface-2 p-0 text-fg shadow-2xl backdrop:bg-black/70"
    >
      <div className="p-6">
        <p className="text-2xs font-medium uppercase tracking-wider text-fg-faint">
          Step {step + 1} of {STEPS.length}
        </p>
        <h2 id={titleId} className="mt-1 text-lg font-semibold">
          {step === 0 ? "Welcome to Night Client" : STEPS[step]}
        </h2>
        <div className="mt-4 text-[13px] leading-relaxed text-fg-muted">
          {step === 0 && (
            <Body icon={<Moon size={18} />}>
              <p>A fast, open-source launcher for Minecraft: Java Edition, with Fabric and NeoForge instances and mods from Modrinth.</p>
              <p className="mt-2">
                To play you need a Microsoft account that owns Minecraft: Java Edition. Night Client is not an official Minecraft product and
                is not associated with Mojang Studios or Microsoft.
              </p>
            </Body>
          )}
          {step === 1 && (
            <Body icon={<FolderOpen size={18} />}>
              <p>Game versions, libraries and Java runtimes are shared by all your instances. They can grow to several gigabytes.</p>
              <p className="mt-2 break-all font-mono text-xs text-fg">{info?.dataDir}</p>
              <p className="mt-2">You can move them to another folder or drive at any time in Settings → Storage.</p>
            </Body>
          )}
          {step === 2 && (
            <Body icon={<ShieldCheck size={18} />}>
              <p>Night Client has no analytics or telemetry. It connects to:</p>
              <ul className="mt-2 list-disc space-y-1 pl-5">
                <li>Microsoft and Xbox Live, to sign you in. Your password is entered on Microsoft's page, never in Night Client.</li>
                <li>Mojang, Fabric and NeoForge, to download game files, which are verified before use.</li>
                <li>Modrinth, only when you browse, install or update mods.</li>
              </ul>
              <p className="mt-2">Crash reports stay on this PC unless you export them from Diagnostics.</p>
              <div className="mt-4">
                <Toggle
                  id={toggleId}
                  checked={checkUpdates}
                  onChange={setCheckUpdates}
                  label="Check for Night Client updates automatically"
                  description="Asks the release feed for a newer version shortly after start. The request carries your Night Client version and Windows architecture, and, like any connection, your IP address."
                />
              </div>
            </Body>
          )}
          {step === 3 && (
            <Body icon={<Boxes size={18} />}>
              <p>Sign in with Microsoft, then create an instance: a separate game setup with its own version, loader, mods and worlds.</p>
              <div className="mt-4 flex flex-wrap gap-2">
                <Button icon={<UserRound size={14} />} onClick={() => void finish({ name: "accounts" })}>
                  Sign in with Microsoft
                </Button>
                <Button icon={<Boxes size={14} />} onClick={() => void finish({ name: "new-instance" })}>
                  Create an instance
                </Button>
              </div>
            </Body>
          )}
        </div>
      </div>
      <div className="flex items-center justify-between gap-2 border-t border-line px-6 py-3">
        <Button variant="ghost" onClick={() => void finish()}>
          {last ? "Close" : "Skip"}
        </Button>
        <div className="flex gap-2">
          {step > 0 && <Button onClick={() => setStep(step - 1)}>Back</Button>}
          {!last && (
            <Button ref={nextRef} variant="primary" onClick={() => setStep(step + 1)}>
              Next
            </Button>
          )}
        </div>
      </div>
    </dialog>
  );
}

function Body({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="flex gap-4">
      <span aria-hidden className="mt-0.5 shrink-0 text-accent">
        {icon}
      </span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}
