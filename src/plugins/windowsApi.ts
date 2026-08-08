import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { assertCapability } from "./capGate";
import type { HubWindowInfo, PluginManifest } from "./types";

export async function listWindows(manifest: PluginManifest): Promise<HubWindowInfo[]> {
  assertCapability(manifest, "windows.read");
  return invoke<HubWindowInfo[]>("hub_windows_list", { pluginId: manifest.id });
}

export async function getWindow(
  manifest: PluginManifest,
  id: string,
): Promise<HubWindowInfo | null> {
  assertCapability(manifest, "windows.read");
  return invoke<HubWindowInfo | null>("hub_windows_get", {
    pluginId: manifest.id,
    id,
  });
}

export async function focusWindow(
  manifest: PluginManifest,
  id: string,
): Promise<void> {
  assertCapability(manifest, "windows.focus");
  await invoke("hub_windows_focus", { pluginId: manifest.id, id });
}

/**
 * Subscribe to the shared WindowsService snapshot (one poller in Rust).
 */
export function subscribeWindows(
  manifest: PluginManifest,
  cb: (windows: HubWindowInfo[]) => void,
): () => void {
  assertCapability(manifest, "windows.read");
  let cancelled = false;
  let unlisten: (() => void) | undefined;

  void listWindows(manifest)
    .then((wins) => {
      if (!cancelled) cb(wins);
    })
    .catch((err) => console.error("[hub.windows.subscribe]", err));

  void listen<{ windows: HubWindowInfo[] }>("hub-windows-changed", (ev) => {
    if (!cancelled && ev.payload?.windows) cb(ev.payload.windows);
  }).then((fn) => {
    unlisten = fn;
  });

  return () => {
    cancelled = true;
    unlisten?.();
  };
}
