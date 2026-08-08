/** Resolve island slot winners from PluginRegistry (no hardcoded plugin ids). */

import { pluginRegistry } from "./registry";
import type { IslandBarState, PluginManifest } from "./types";

export type { IslandBarState };

function orderOf(manifest: PluginManifest, slot: "island.bar" | "island.drop"): number {
  return manifest.slots?.[slot]?.order ?? 100;
}

/** Enabled plugins declaring island.drop (+ staging recommended for transfer UX). */
export function listIslandDropPlugins() {
  return pluginRegistry
    .listAll()
    .filter(
      (p) =>
        p.enabled &&
        p.manifest.slots?.["island.drop"] &&
        (p.manifest.capabilities ?? []).includes("island.drop") &&
        (p.manifest.capabilities ?? []).includes("staging"),
    )
    .sort(
      (a, b) =>
        orderOf(a.manifest, "island.drop") - orderOf(b.manifest, "island.drop"),
    );
}

/** Single drop target (lowest order). */
export function resolveIslandDropPluginId(): string | null {
  return listIslandDropPlugins()[0]?.pluginId ?? null;
}

export function listIslandBarPlugins() {
  return pluginRegistry
    .listAll()
    .filter((p) => p.enabled && p.manifest.slots?.["island.bar"])
    .sort(
      (a, b) => orderOf(a.manifest, "island.bar") - orderOf(b.manifest, "island.bar"),
    );
}

export function getPluginManifest(pluginId: string): PluginManifest | undefined {
  return pluginRegistry.get(pluginId)?.manifest;
}

/** Panel defaultSize from slot declaration (settings.panelWidth/Height override at runtime). */
export function resolvePanelDefaultSize(
  pluginId: string | null | undefined,
): { w?: number; h?: number } {
  if (!pluginId) return {};
  return getPluginManifest(pluginId)?.slots?.["island.panel"]?.defaultSize ?? {};
}

export function formatStagingBarText(
  pluginName: string,
  s: { files: number; texts: number; images: number; total: number },
): string {
  if (!s.total) return "";
  const parts = [pluginName];
  if (s.files > 0) parts.push(`文件 ${s.files}`);
  if (s.texts > 0) parts.push(`文字片段 ${s.texts}`);
  if (s.images > 0) parts.push(`图片 ${s.images}`);
  return parts.join(" | ");
}
