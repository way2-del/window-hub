import type {
  PluginCapability,
  PluginManifest,
  ShortcutItem,
  ShortcutsPluginRuntime,
} from "./types";
import { isPluginSurfaceEnabled } from "./surfacePrefs";

export type InstalledPluginMeta = {
  id: string;
  name: string;
  version: string;
  path: string;
  enabled: boolean;
  isDev: boolean;
  capabilities: PluginCapability[];
  manifest: PluginManifest;
};

type Listener = () => void;

type Runtime = ShortcutsPluginRuntime & {
  installPath?: string;
  enabled: boolean;
};

class PluginRegistry {
  private plugins = new Map<string, Runtime>();
  private listeners = new Set<Listener>();

  register(
    manifest: PluginManifest,
    opts?: { installPath?: string; enabled?: boolean },
  ) {
    const prev = this.plugins.get(manifest.id);
    this.plugins.set(manifest.id, {
      pluginId: manifest.id,
      manifest,
      badge: prev?.badge ?? null,
      items: prev?.items ?? [],
      expanded: prev?.expanded ?? false,
      installPath: opts?.installPath ?? prev?.installPath,
      enabled: opts?.enabled ?? prev?.enabled ?? true,
    });
    this.emit();
  }

  unregister(pluginId: string) {
    this.plugins.delete(pluginId);
    this.emit();
  }

  setEnabled(pluginId: string, enabled: boolean) {
    const p = this.plugins.get(pluginId);
    if (!p) return;
    p.enabled = enabled;
    this.emit();
  }

  listAll(): Runtime[] {
    return [...this.plugins.values()];
  }

  listShortcuts(): ShortcutsPluginRuntime[] {
    return [...this.plugins.values()]
      .filter(
        (p) =>
          p.enabled &&
          p.manifest.slots?.shortcuts &&
          isPluginSurfaceEnabled(p.pluginId, "shortcuts", p.manifest),
      )
      .sort(
        (a, b) =>
          (a.manifest.slots!.shortcuts!.order ?? 100) -
          (b.manifest.slots!.shortcuts!.order ?? 100),
      );
  }

  listPanelManifests(): PluginManifest[] {
    return [...this.plugins.values()]
      .filter((p) => p.enabled && p.manifest.slots?.["island.panel"])
      .map((p) => p.manifest);
  }

  listNotifyManifests(): PluginManifest[] {
    return [...this.plugins.values()]
      .filter((p) => p.enabled && p.manifest.slots?.["island.notify"])
      .map((p) => p.manifest);
  }

  get(pluginId: string): Runtime | undefined {
    return this.plugins.get(pluginId);
  }

  setBadge(pluginId: string, badge: string | number | null) {
    const p = this.plugins.get(pluginId);
    if (!p) return;
    p.badge = badge;
    this.emit();
  }

  setItems(pluginId: string, items: ShortcutItem[]) {
    const p = this.plugins.get(pluginId);
    if (!p) return;
    p.items = items;
    this.emit();
  }

  setExpanded(pluginId: string | null) {
    for (const p of this.plugins.values()) {
      p.expanded = pluginId != null && p.pluginId === pluginId;
    }
    this.emit();
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit() {
    for (const l of this.listeners) l();
  }
}

export const pluginRegistry = new PluginRegistry();
