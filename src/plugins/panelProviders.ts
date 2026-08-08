import type { PluginManifest } from "./types";

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

const BUILTIN: PanelProvider[] = [
  {
    id: "weather",
    label: "天气",
    description: "温度、湿度与风力详情",
    kind: "builtin",
  },
  {
    id: "mirror",
    label: "镜子",
    description: "摄像头预览",
    kind: "builtin",
  },
];

/** Resolve pullContent id list for settings + island.
 * `excludeFromPullContent` plugins open via drop / bar / session only.
 * Shell width etc. via plugin settings — not prefs_island.
 */
export function listPanelProviders(
  panelPlugins: PluginManifest[],
): PanelProvider[] {
  const fromPlugins: PanelProvider[] = panelPlugins
    .filter(
      (m) =>
        m.slots?.["island.panel"] &&
        !m.slots["island.panel"]?.excludeFromPullContent &&
        (m.entry?.panel || m.entry?.development?.panel),
    )
    .map((m) => ({
      id: `plugin:${m.id}`,
      label: m.name,
      description: m.description ?? "插件面板",
      kind: "plugin" as const,
      pluginId: m.id,
      manifest: m,
    }));
  return [...BUILTIN, ...fromPlugins];
}

export function isBuiltinPanel(id: string): id is "weather" | "mirror" {
  return id === "weather" || id === "mirror";
}

export function parsePluginPanelId(pullContent: string): string | null {
  if (!pullContent.startsWith("plugin:")) return null;
  return pullContent.slice("plugin:".length) || null;
}
