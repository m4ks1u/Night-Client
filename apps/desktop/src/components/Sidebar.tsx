import { Activity, Boxes, Download, House, Settings, UserRound } from "lucide-react";
import type { ReactNode } from "react";
import { navigate, sectionOf, useRouter, type Route } from "../stores/router";
import { useApp } from "../stores/app";

type Item = { route: Route; label: string; icon: ReactNode };

const main: Item[] = [
  { route: { name: "home" }, label: "Home", icon: <House size={17} strokeWidth={1.75} /> },
  { route: { name: "instances" }, label: "Instances", icon: <Boxes size={17} strokeWidth={1.75} /> },
  { route: { name: "downloads" }, label: "Downloads", icon: <Download size={17} strokeWidth={1.75} /> },
];

const secondary: Item[] = [
  { route: { name: "settings" }, label: "Settings", icon: <Settings size={17} strokeWidth={1.75} /> },
  { route: { name: "diagnostics" }, label: "Diagnostics", icon: <Activity size={17} strokeWidth={1.75} /> },
];

export function Sidebar() {
  const section = useRouter((s) => sectionOf(s.route));
  const account = useApp((s) => s.account);
  const runningCount = useApp((s) => Object.keys(s.running).length);
  const downloads = useApp((s) => (s.queue.active ? 1 : 0) + s.queue.pending.length);

  return (
    <nav aria-label="Main" className="flex w-[216px] shrink-0 flex-col border-r border-line bg-surface-1 px-3 py-4">
      <ul className="flex flex-col gap-0.5">
        {main.map((i) => (
          <NavItem key={i.label} item={i} active={section === i.route.name}>
            {i.route.name === "downloads" && downloads > 0 && (
              <span className="ml-auto grid h-4 min-w-4 place-items-center rounded-full bg-accent px-1 text-2xs font-semibold text-accent-fg" aria-label={`${downloads} active`}>
                {downloads}
              </span>
            )}
            {i.route.name === "instances" && runningCount > 0 && (
              <span className="ml-auto flex items-center gap-1.5 text-2xs text-success">
                <span className="size-1.5 rounded-full bg-success" aria-hidden="true" />
                {runningCount} running
              </span>
            )}
          </NavItem>
        ))}
      </ul>


      <ul className="mt-auto flex flex-col gap-0.5">
        {secondary.map((i) => (
          <NavItem key={i.label} item={i} active={section === i.route.name} />
        ))}
      </ul>

      <button
        type="button"
        onClick={() => navigate({ name: "accounts" })}
        aria-current={section === "accounts" ? "page" : undefined}
        className={`mt-3 flex items-center gap-2.5 rounded-md border px-2.5 py-2 text-left transition-colors duration-(--duration-fast) ${
          section === "accounts" ? "border-line-strong bg-surface-3" : "border-line hover:border-line-strong hover:bg-surface-2"
        }`}
      >
        <span className="grid size-7 shrink-0 place-items-center rounded-full bg-surface-3 text-fg-muted">
          <UserRound size={15} />
        </span>
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium">{account ? account.name : "Not signed in"}</span>
          <span className={`block text-2xs ${account?.status === "expired" ? "text-warning" : "text-fg-faint"}`}>
            {!account ? "Sign in to play" : account.status === "expired" ? "Sign in again" : "Microsoft account"}
          </span>
        </span>
      </button>
    </nav>
  );
}

function NavItem({ item, active, children }: { item: Item; active: boolean; children?: ReactNode }) {
  return (
    <li>
      <button
        type="button"
        aria-current={active ? "page" : undefined}
        onClick={() => navigate(item.route)}
        className={`relative flex h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px] font-medium transition-colors duration-(--duration-fast) ${
          active ? "bg-surface-3 text-fg" : "text-fg-muted hover:bg-surface-2 hover:text-fg"
        }`}
      >
        {active && <span className="absolute top-2 bottom-2 left-0 w-0.5 rounded-full bg-accent" aria-hidden="true" />}
        <span className={active ? "text-accent" : ""}>{item.icon}</span>
        {item.label}
        {children}
      </button>
    </li>
  );
}
