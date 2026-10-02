import type { PluginManifest } from "./types";
import { isPluginSurfaceEnabled } from "./surfacePrefs";

export type PanelProvider = {
  id: string;
  label: string;
  description: string;
  /** Builtin React panel or plugin */
  kind: "builtin" | "plugin";
  pluginId?: string;
  /** Absolute path or convertFileSrc URL for iframe */
  entryUrl?: string;
  manifest?: PluginManifest;
};

/**
 * Resolve pullContent id list for settings + island.
 * Weather / mirror are plugins — no Host builtin entries.
 * `excludeFromPullContent` plugins open via drop / bar / session only.
 * Prefer `listDashboardPanelProviders` + `resolvePanelPullMode` for Host home.
 */
export function listPanelProviders(
  panelPlugins: PluginManifest[],
): PanelProvider[] {
  return panelPlugins
    .filter(
      (m) =>
        m.slots?.["island.panel"] &&
        !m.slots["island.panel"]?.excludeFromPullContent &&
        !m.slots?.["island.scenario"] &&
        (m.entry?.panel || m.entry?.development?.panel) &&
        isPluginSurfaceEnabled(m.id, "island.panel", m),
    )
    .map((m) => ({
      id: `plugin:${m.id}`,
      label: m.name,
      description: m.description ?? "插件面板",
      kind: "plugin" as const,
      pluginId: m.id,
      manifest: m,
    }));
}

/** @deprecated Host no longer ships builtin weather/mirror panels */
export function isBuiltinPanel(_id: string): boolean {
  return false;
}

export function parsePluginPanelId(pullContent: string): string | null {
  if (!pullContent.startsWith("plugin:")) return null;
  return pullContent.slice("plugin:".length) || null;
}
