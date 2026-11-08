import { useEffect, useRef, useState } from "react";
import { AnimatePresence, LazyMotion, MotionConfig, domAnimation, m } from "motion/react";
import { DatabaseZap, WifiOff } from "lucide-react";

import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { Toaster } from "./components/Toaster";
import { CommandPalette } from "./components/CommandPalette";
import { api, events } from "./lib/ipc";
import { labelForKey } from "./lib/keys";
import { useApp } from "./stores/app";
import { navigate, useRouter, type Route } from "./stores/router";
import { toast } from "./stores/toast";
import { Home } from "./screens/Home";
import { Instances } from "./screens/Instances";
import { NewInstance } from "./screens/NewInstance";
import { InstanceDetail } from "./screens/InstanceDetail";
import { Accounts } from "./screens/Accounts";
import { Downloads } from "./screens/Downloads";
import { Discover } from "./screens/Discover";
import { Settings } from "./screens/Settings";
import { Diagnostics } from "./screens/Diagnostics";
import { Welcome } from "./components/Welcome";

function Screen({ route }: { route: Route }) {
  switch (route.name) {
    case "home":
      return <Home />;
    case "instances":
      return <Instances />;
    case "new-instance":
      return <NewInstance />;
    case "instance":
      return <InstanceDetail id={route.id} tab={route.tab} />;
    case "discover":
      return <Discover instanceId={route.instanceId} />;
    case "downloads":
      return <Downloads />;
    case "accounts":
      return <Accounts />;
    case "settings":
      return <Settings />;
    case "diagnostics":
      return <Diagnostics />;
  }
}

function routeKey(r: Route) {
  return r.name === "instance" ? `instance:${r.id}:${r.tab ?? "overview"}` : r.name;
}

function useOnline() {
  const [online, setOnline] = useState(navigator.onLine);
  useEffect(() => {
    const up = () => setOnline(true);
    const down = () => setOnline(false);
    window.addEventListener("online", up);
    window.addEventListener("offline", down);
    return () => {
      window.removeEventListener("online", up);
      window.removeEventListener("offline", down);
    };
  }, []);
  return online;
}

/** Background results the user may not be looking at become toasts. */
function subscribe() {
  return [
    events.installProgress((p) => useApp.getState().setProgress(p)),
    events.downloadsQueue((q) => useApp.getState().setQueue(q)),
    events.installFinished((f) => {
      useApp.getState().onFinished(f);
      const name = labelForKey(f.id);
      if (f.status === "completed") {
        toast(
          f.repair
            ? { tone: f.repairedCount > 0 ? "info" : "success", title: `${name} verified`, body: f.repairedCount > 0 ? `${f.repairedCount} damaged or missing files were repaired.` : "Every file matches." }
            : { tone: "success", title: `${name} is ready to play` },
        );
      } else if (f.status === "failed") {
        toast({ tone: "error", title: `${name} could not be installed`, body: f.error ?? undefined, action: { label: "View downloads", route: { name: "downloads" } } });
      }
    }),
    events.gameStarted((id) => useApp.getState().markStarted(id)),
    events.gameLog((b) => useApp.getState().appendLogs(b.instanceId, b.lines)),
    events.gameExit((e) => {
      useApp.getState().markExited(e.instanceId);
      void useApp.getState().refreshInstances();
      if (e.code !== 0 && e.code !== null) {
        const s = useApp.getState().instances;
        const name = s.status === "ready" ? (s.data.find((i) => i.id === e.instanceId)?.name ?? "The game") : "The game";
        toast({ tone: "error", title: `${name} closed with an error`, body: `Exit code ${e.code}.`, action: { label: "View game log", route: { name: "instance", id: e.instanceId, tab: "log" } } });
      }
    }),
    events.updateAvailable((version) =>
      toast({ tone: "info", title: `Night Client ${version} is available`, body: "Install it from Settings when you're ready.", action: { label: "Open Settings", route: { name: "settings" } } }),
    ),
  ];
}

export default function App() {
  const route = useRouter((s) => s.route);
  const back = useRouter((s) => s.back);
  const startupError = useApp((s) => s.info?.startupError);
  const reduceMotion = useApp((s) => s.settings?.reduceMotion ?? false);
  const compact = useApp((s) => s.settings?.compactLayout ?? false);
  const online = useOnline();
  const [palette, setPalette] = useState(false);
  const mainRef = useRef<HTMLElement>(null);
  const firstRoute = useRef(true);

  useEffect(() => {
    void useApp.getState().boot();
    const subs = subscribe();
    api
      .takeCrashNotice()
      .then((c) => {
        if (c.count > 0)
          toast({
            tone: "error",
            title: "Night Client closed unexpectedly last time",
            body: "A crash report was saved on this PC. You can export it from Diagnostics to report the problem.",
            action: { label: "Open Diagnostics", route: { name: "diagnostics" } },
          });
      })
      .catch(() => {});
    const onKey = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      if (e.altKey && e.key === "ArrowLeft") back();
      else if (ctrl && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      } else if (ctrl && e.key.toLowerCase() === "n") {
        e.preventDefault();
        navigate({ name: "new-instance" });
      } else if (ctrl && e.key === ",") {
        e.preventDefault();
        navigate({ name: "settings" });
      } else if (ctrl && e.key.toLowerCase() === "f") {
        const search = document.querySelector<HTMLInputElement>("main input[type=search]");
        if (search) {
          e.preventDefault();
          search.focus();
          search.select();
        }
      }
    };
    // Mouse "back" button.
    const onMouse = (e: MouseEvent) => e.button === 3 && back();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      subs.forEach((p) => void p.then((off) => off()));
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [back]);

  useEffect(() => {
    document.documentElement.classList.toggle("reduce-motion", reduceMotion);
  }, [reduceMotion]);

  useEffect(() => {
    document.documentElement.classList.toggle("density-compact", compact);
  }, [compact]);

  // Move focus to the new page's heading so keyboard and screen-reader users
  // land on the new content (not on the first launch).
  useEffect(() => {
    if (firstRoute.current) {
      firstRoute.current = false;
      return;
    }
    const t = setTimeout(() => {
      const h = mainRef.current?.querySelector<HTMLElement>("h1");
      if (h) {
        h.tabIndex = -1;
        h.focus({ preventScroll: true });
      }
    }, 180);
    return () => clearTimeout(t);
  }, [route]);

  return (
    <MotionConfig reducedMotion={reduceMotion ? "always" : "user"}>
      <LazyMotion features={domAnimation} strict>
        <a
          href="#main"
          onClick={(e) => {
            e.preventDefault();
            mainRef.current?.querySelector<HTMLElement>("h1")?.focus();
          }}
          className="sr-only z-50 rounded-md bg-accent px-3 py-2 text-accent-fg focus:not-sr-only focus:fixed focus:top-2 focus:left-2"
        >
          Skip to content
        </a>
        <div className="flex h-full flex-col">
          <TitleBar />
          {startupError && (
            <div role="alert" className="flex items-center gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-xs text-danger">
              <DatabaseZap size={14} />
              <span className="select-text">Night Client could not open its database: {startupError}</span>
            </div>
          )}
          {!online && (
            <div role="status" className="flex items-center gap-2 border-b border-warning/30 px-4 py-2 text-xs text-warning">
              <WifiOff size={14} />
              You're offline. Installed instances still work; downloads, sign-in and Modrinth need a connection.
            </div>
          )}
          <div className="flex min-h-0 flex-1">
            <Sidebar onOpenPalette={() => setPalette(true)} />
            <main id="main" ref={mainRef} className="min-w-0 flex-1 overflow-y-auto">
              <AnimatePresence mode="wait" initial={false}>
                <m.div
                  key={routeKey(route)}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.14, ease: [0.2, 0.8, 0.2, 1] }}
                  className="mx-auto flex h-full max-w-5xl flex-col px-8 py-7"
                >
                  <Screen route={route} />
                </m.div>
              </AnimatePresence>
            </main>
          </div>
        </div>
        <Toaster />
        <CommandPalette open={palette} onClose={() => setPalette(false)} />
        <Welcome />
      </LazyMotion>
    </MotionConfig>
  );
}
