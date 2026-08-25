import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { pluginRegistry } from "./registry";
import type { PluginCapability, PluginManifest } from "./types";
import { refreshSurfaceSettingsCache } from "./surfacePrefs";

export type InstalledPluginDto = {
  id: string;
  name: string;
  version: string;
  path: string;
  enabled: boolean;
  isDev: boolean;
  capabilities: string[];
  manifest: PluginManifest;
};

/** True after a successful list_installed_plugins / plugins-changed apply. */
let pluginsReady = false;

/** Island prefs must not treat "missing from empty registry" as disabled until ready. */
export function arePluginsReady(): boolean {
  return pluginsReady;
}

function applyInstalled(list: InstalledPluginDto[]) {
  // Drop entries that disappeared from the backend registry
  for (const p of pluginRegistry.listAll()) {
    if (!list.some((x) => x.id === p.pluginId)) {
      pluginRegistry.unregister(p.pluginId);
    }
  }
  for (const item of list) {
    const caps = (item.manifest.capabilities ?? item.capabilities) as PluginCapability[];
    pluginRegistry.register(
      { ...item.manifest, id: item.id, capabilities: caps },
      { installPath: item.path, enabled: item.enabled },
    );
  }
  pluginsReady = true;
}

export async function bootstrapPlugins(): Promise<InstalledPluginDto[]> {
  try {
    const list = await invoke<InstalledPluginDto[]>("list_installed_plugins");
  applyInstalled(list);
  void refreshSurfaceSettingsCache(list.map((p) => p.id));
  return list;
  } catch (err) {
    console.error("[bootstrapPlugins]", err);
    // Leave pluginsReady false so startup sync does not wipe island prefs.
    return [];
  }
}

export async function subscribeInstalledPlugins(
  onChange?: (list: InstalledPluginDto[]) => void,
): Promise<() => void> {
  const unlisten = await listen<InstalledPluginDto[]>("plugins-changed", (ev) => {
    applyInstalled(ev.payload);
    onChange?.(ev.payload);
  });
  return () => unlisten();
}
