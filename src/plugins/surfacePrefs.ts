/** Multi-surface enablement via plugin settings `enabledSurfaces` (multiSelect). */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PluginManifest } from "./types";

export type PluginSurfaceKey = "shortcuts" | "island.bar" | "island.panel";

const SLOT_TO_SURFACE: Record<string, PluginSurfaceKey> = {
  shortcuts: "shortcuts",
  "island.bar": "island.bar",
  "island.panel": "island.panel",
};

const settingsCache = new Map<string, Record<string, unknown>>();
let cacheReady = false;

export function declaredSurfaces(manifest: PluginManifest): PluginSurfaceKey[] {
  const slots = manifest.slots ?? {};
  const out: PluginSurfaceKey[] = [];
  for (const [slot, surface] of Object.entries(SLOT_TO_SURFACE)) {
    if (slots[slot as keyof typeof slots]) out.push(surface);
  }
  return out;
}

export function parseEnabledSurfaces(
  settings: Record<string, unknown> | null | undefined,
  manifest: PluginManifest,
): PluginSurfaceKey[] {
  const declared = declaredSurfaces(manifest);
  if (!declared.length) return [];
  const raw = settings?.enabledSurfaces;
  if (!Array.isArray(raw) || raw.length === 0) return declared;
  const allowed = new Set(declared);
  // Drop unknown/legacy keys (e.g. removed "desktop"); if nothing valid remains,
  // fall back to all declared surfaces so Host settings stay usable.
  const filtered = raw.filter(
    (v): v is PluginSurfaceKey => typeof v === "string" && allowed.has(v as PluginSurfaceKey),
  );
  return filtered.length > 0 ? filtered : declared;
}

export function isSurfaceEnabled(
  settings: Record<string, unknown> | null | undefined,
  surface: PluginSurfaceKey,
  manifest: PluginManifest,
): boolean {
  if (!declaredSurfaces(manifest).includes(surface)) return false;
  return parseEnabledSurfaces(settings, manifest).includes(surface);
}

export function cachedPluginSettings(pluginId: string): Record<string, unknown> {
  return settingsCache.get(pluginId) ?? {};
}

export function isPluginSurfaceEnabled(
  pluginId: string,
  surface: PluginSurfaceKey,
  manifest: PluginManifest,
): boolean {
  return isSurfaceEnabled(cachedPluginSettings(pluginId), surface, manifest);
}

export async function refreshSurfaceSettingsCache(pluginIds?: string[]): Promise<void> {
  const ids =
    pluginIds ??
    (await invoke<Array<{ id: string }>>("list_installed_plugins").catch(() => [])).map((p) => p.id);
  await Promise.all(
    ids.map(async (id) => {
      try {
        const all = await invoke<Record<string, unknown>>("hub_settings_get_all", { pluginId: id });
        settingsCache.set(id, all ?? {});
      } catch {
        settingsCache.delete(id);
      }
    }),
  );
  cacheReady = true;
}

export function isSurfaceSettingsCacheReady(): boolean {
  return cacheReady;
}

export function subscribeSurfaceSettingsCache(onChange?: () => void): () => void {
  let un: (() => void) | undefined;
  void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
    "plugin-settings-changed",
    (ev) => {
      const id = ev.payload?.pluginId;
      if (!id) return;
      settingsCache.set(id, ev.payload?.settings ?? {});
      onChange?.();
    },
  ).then((fn) => {
    un = fn;
  });
  return () => un?.();
}

/** Default multiSelect options for plugins declaring multiple surfaces. */
export function enabledSurfacesSettingOptions(manifest: PluginManifest) {
  const labels: Record<PluginSurfaceKey, string> = {
    shortcuts: "快捷区",
    "island.bar": "岛栏摘要",
    "island.panel": "岛下拉",
  };
  return declaredSurfaces(manifest).map((s) => ({
    value: s,
    label: labels[s],
  }));
}
