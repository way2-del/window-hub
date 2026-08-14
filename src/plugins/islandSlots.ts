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

/** 全局设置「岛栏常驻」候选：已启用 + island.bar，且非 excludeFromBarResident、非情景临时 */
export function listBarResidentProviders(): {
  id: string;
  label: string;
  description: string;
}[] {
  return listIslandBarPlugins()
    .filter(
      (p) =>
        !p.manifest.slots?.["island.bar"]?.excludeFromBarResident &&
        !p.manifest.slots?.["island.scenario"],
    )
    .map((p) => ({
      id: p.pluginId,
      label: p.manifest.name,
      description: p.manifest.description ?? "岛栏摘要",
    }));
}

/** 已启用的情景临时插件（只读列表用） */
export function listScenarioProviders(): {
  id: string;
  label: string;
  description: string;
}[] {
  return pluginRegistry
    .listAll()
    .filter(
      (p) =>
        p.enabled &&
        p.manifest.slots?.["island.scenario"] &&
        p.manifest.slots?.["island.bar"] &&
        p.manifest.slots?.["island.panel"] &&
        (p.manifest.capabilities ?? []).includes("island.bar") &&
        (p.manifest.capabilities ?? []).includes("island.panel"),
    )
    .sort((a, b) => {
      const oa = a.manifest.slots?.["island.scenario"]?.order ?? 100;
      const ob = b.manifest.slots?.["island.scenario"]?.order ?? 100;
      return oa - ob;
    })
    .map((p) => ({
      id: p.pluginId,
      label: p.manifest.name,
      description: p.manifest.description ?? "情景临时接管岛栏与下拉",
    }));
}

export function pluginHasScenario(pluginId: string): boolean {
  return Boolean(pluginRegistry.get(pluginId)?.manifest.slots?.["island.scenario"]);
}

export function getPluginManifest(pluginId: string): PluginManifest | undefined {
  return pluginRegistry.get(pluginId)?.manifest;
}

/** 岛栏折叠宽自适应（slots.island.bar.adaptiveWidth） */
export type IslandBarAdaptive = {
  enabled: boolean;
  minWidth: number;
  maxWidth: number;
};

const ADAPTIVE_MIN_DEFAULT = 220;
const ADAPTIVE_MAX_DEFAULT = 560;

export function resolveIslandBarAdaptive(
  pluginId: string | null | undefined,
): IslandBarAdaptive {
  if (!pluginId) {
    return { enabled: false, minWidth: ADAPTIVE_MIN_DEFAULT, maxWidth: ADAPTIVE_MAX_DEFAULT };
  }
  const slot = getPluginManifest(pluginId)?.slots?.["island.bar"];
  const enabled = Boolean(slot?.adaptiveWidth);
  const minWidth = Math.min(
    ADAPTIVE_MAX_DEFAULT,
    Math.max(160, Number(slot?.minWidth) || ADAPTIVE_MIN_DEFAULT),
  );
  const maxWidth = Math.max(
    minWidth,
    Math.min(720, Number(slot?.maxWidth) || ADAPTIVE_MAX_DEFAULT),
  );
  return { enabled, minWidth, maxWidth };
}

/** Measure island bar label width (matches .bar-staging-text chrome). */
export function measureIslandBarLabelWidth(
  text: string,
  opts?: { showDot?: boolean },
): number {
  const t = String(text || "");
  if (!t) return 0;
  if (typeof document === "undefined") {
    return Math.ceil(t.length * 9);
  }
  const canvas =
    (measureIslandBarLabelWidth as unknown as { _c?: HTMLCanvasElement })._c ??
    document.createElement("canvas");
  (measureIslandBarLabelWidth as unknown as { _c?: HTMLCanvasElement })._c = canvas;
  const ctx = canvas.getContext("2d");
  if (!ctx) return Math.ceil(t.length * 9);
  // Match .bar-staging-text: 12px / 600 + letter-spacing 0.01em
  ctx.font =
    '600 12px "Segoe UI", "PingFang SC", "Microsoft YaHei UI", "Microsoft YaHei", system-ui, sans-serif';
  const tw = ctx.measureText(t).width + t.length * 0.12;
  const chromePad = 32; // 岛左右内边距 + 安全余量
  const dot = opts?.showDot ? 13 : 0;
  return Math.ceil(tw + chromePad + dot);
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
  s: { files: number; texts: number; images: number; total: number },
): string {
  if (!s.total) return "";
  const parts = [pluginName];
  if (s.files > 0) parts.push(`文件 ${s.files}`);
  if (s.texts > 0) parts.push(`文字片段 ${s.texts}`);
  if (s.images > 0) parts.push(`图片 ${s.images}`);
  return parts.join(" | ");
}
