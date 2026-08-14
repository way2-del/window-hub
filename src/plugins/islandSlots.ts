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
    .filter(
      (p) =>
        p.enabled &&
        p.manifest.slots?.["island.bar"] &&
        (p.manifest.capabilities ?? []).includes("island.bar"),
    )
    .sort(
      (a, b) => orderOf(a.manifest, "island.bar") - orderOf(b.manifest, "island.bar"),
    );
}

/** 全局设置「岛栏常驻」候选：已启用 + island.bar 能力/槽位，且未 excludeFromBarResident */
export function listBarResidentProviders(): {
  id: string;
  label: string;
  description: string;
}[] {
  return listIslandBarPlugins()
    .filter((p) => !p.manifest.slots?.["island.bar"]?.excludeFromBarResident)
    .map((p) => ({
      id: p.pluginId,
      label: p.manifest.name,
      description: p.manifest.description ?? "岛栏摘要",
    }));
}

/**
 * 折叠岛栏内容竞选（通知横幅由 Host 另层处理）：
 * 按 settings.barPriority 顺序取第一条有文案的插件。
 * 临时层（中转站 excludeFromBarResident）在 App 侧盖在本结果之上。
 */
export function pickIslandContentBar(
  bars: ReadonlyMap<string, IslandBarState>,
  barPriority: string[],
): IslandBarState | null {
  const textOk = (b: IslandBarState | undefined) =>
    !!b && (!!String(b.text ?? "").trim() || !!String(b.image ?? "").trim());
  const seen = new Set<string>();
  for (const id of barPriority) {
    const key = String(id ?? "").trim();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    const hit = bars.get(key);
    if (textOk(hit)) return hit!;
  }
  // 未写入顺序的插件：按 slot.order 兜底
  for (const p of listIslandBarPlugins()) {
    if (p.manifest.slots?.["island.bar"]?.excludeFromBarResident) continue;
    if (seen.has(p.pluginId)) continue;
    const hit = bars.get(p.pluginId);
    if (textOk(hit)) return hit!;
  }
  return null;
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

/** Weather-like panel hard top (non-transfer). */
export const PANEL_VIEW_W_DEFAULT = 380;
export const PANEL_VIEW_H_DEFAULT = 220;

/**
 * Resolve plugin island panel shell size.
 * - If settings declare panelWidth/panelHeight → staging clamps (中转站)
 * - Else honor slots.defaultSize, fallback 380×220 (备忘/天气型)
 * Never force weather-sized plugins through staging 440–720×120–184 clamps.
 */
export function resolvePluginPanelShellSize(
  pluginId: string,
  settings?: Record<string, unknown> | null,
  clampStaging?: {
    w: (n: number) => number;
    h: (n: number) => number;
  },
): { w: number; h: number } {
  const defaults = resolvePanelDefaultSize(pluginId);
  const hasStagingKeys =
    !!settings &&
    (settings.panelWidth != null || settings.panelHeight != null);

  if (hasStagingKeys && clampStaging) {
    return {
      w: clampStaging.w(
        Number(settings?.panelWidth ?? defaults.w ?? 560),
      ),
      h: clampStaging.h(
        Number(settings?.panelHeight ?? defaults.h ?? 152),
      ),
    };
  }

  const w = Math.round(Number(defaults.w ?? PANEL_VIEW_W_DEFAULT));
  const h = Math.round(Number(defaults.h ?? PANEL_VIEW_H_DEFAULT));
  return {
    w: Math.max(280, Math.min(720, Number.isFinite(w) ? w : PANEL_VIEW_W_DEFAULT)),
    h: Math.max(120, Math.min(320, Number.isFinite(h) ? h : PANEL_VIEW_H_DEFAULT)),
  };
}

/** Staging / transfer-style short-wide shell (uses is-plugin-sized padding). */
export function isStagingPanelShell(w: number, h: number): boolean {
  return h <= 184 + 4 && w >= 440 - 4;
}

export function formatStagingBarText(
  pluginName: string,
  s: { files: number; texts: number; images: number; folders?: number; total: number },
): string {
  if (!s.total) return "";
  const parts = [pluginName];
  if (s.files > 0) parts.push(`文件 ${s.files}`);
  if ((s.folders ?? 0) > 0) parts.push(`文件夹 ${s.folders}`);
  if (s.texts > 0) parts.push(`文字片段 ${s.texts}`);
  if (s.images > 0) parts.push(`图片 ${s.images}`);
  return parts.join(" | ");
}
