// User-level flows shared by several screens.

import { api, toCmdError, type CmdError, type Instance } from "./ipc";
import { useApp } from "../stores/app";
import { navigate } from "../stores/router";

/**
 * Play an instance: installs (or repairs) the version first when needed,
 * then launches. Returns an error for the caller to show, or null.
 */
export async function playInstance(i: Instance): Promise<CmdError | null> {
  const app = useApp.getState();
  if (!app.account) {
    navigate({ name: "accounts" });
    return null;
  }
  if (!i.installed) {
    const ok = await app.installVersion(i.gameVersion);
    if (!ok) return useApp.getState().installErrors[i.gameVersion] ?? null;
  }
  useApp.getState().clearLogs(i.id);
  try {
    await api.launchInstance(i.id);
    useApp.getState().markStarted(i.id);
    return null;
  } catch (e) {
    return toCmdError(e);
  }
}
