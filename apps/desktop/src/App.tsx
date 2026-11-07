import { useEffect } from "react";
import { AnimatePresence, LazyMotion, MotionConfig, domAnimation, m } from "motion/react";
import { DatabaseZap } from "lucide-react";

import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { events } from "./lib/ipc";
import { useApp } from "./stores/app";
import { useRouter, type Route } from "./stores/router";
import { Home } from "./screens/Home";
import { Instances } from "./screens/Instances";
import { NewInstance } from "./screens/NewInstance";
import { InstanceDetail } from "./screens/InstanceDetail";
import { Accounts } from "./screens/Accounts";
import { Settings } from "./screens/Settings";
import { Diagnostics } from "./screens/Diagnostics";

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
    case "accounts":
      return <Accounts />;
    case "settings":
      return <Settings />;
    case "diagnostics":
      return <Diagnostics />;
  }
}

function routeKey(r: Route) {
  return r.name === "instance" ? `instance:${r.id}` : r.name;
}

export default function App() {
  const route = useRouter((s) => s.route);
  const back = useRouter((s) => s.back);
  const startupError = useApp((s) => s.info?.startupError);

  useEffect(() => {
    const s = useApp.getState();
    void s.boot();
    const subs = [
      events.installProgress((p) => useApp.getState().setProgress(p)),
      events.gameStarted((id) => useApp.getState().markStarted(id)),
      events.gameLog((b) => useApp.getState().appendLogs(b.instanceId, b.lines)),
      events.gameExit((e) => {
        useApp.getState().markExited(e.instanceId);
        void useApp.getState().refreshInstances();
      }),
    ];
    // Alt+Left / mouse back button navigate back, like other desktop apps.
    const onKey = (e: KeyboardEvent) => e.altKey && e.key === "ArrowLeft" && back();
    const onMouse = (e: MouseEvent) => e.button === 3 && back();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      subs.forEach((p) => void p.then((off) => off()));
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [back]);

  return (
    <MotionConfig reducedMotion="user">
      <LazyMotion features={domAnimation} strict>
        <div className="flex h-full flex-col">
          <TitleBar />
          {startupError && (
            <div role="alert" className="flex items-center gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-xs text-danger">
              <DatabaseZap size={14} />
              <span className="select-text">Night Client could not open its database: {startupError}</span>
            </div>
          )}
          <div className="flex min-h-0 flex-1">
            <Sidebar />
            <main className="min-w-0 flex-1 overflow-y-auto">
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
      </LazyMotion>
    </MotionConfig>
  );
}
