import type { PluginManifest } from "./types";

export type PanelPullMode = "dashboard" | "standalone";

export type DashboardPanelProvider = {
  id: string;
  label: string;
  description: string;
  order: number;
  manifest: PluginManifest;
};

/**
 * Resolve how a panel plugin presents on island pull.
 * Explicit `pullMode` wins; else excludeFromPullContent → standalone, else dashboard.
 */
export function resolvePanelPullMode(
  manifest: PluginManifest | null | undefined,
): PanelPullMode {
  const slot = manifest?.slots?.["island.panel"];
  if (!slot) return "standalone";
  if (slot.pullMode === "dashboard" || slot.pullMode === "standalone") {
    return slot.pullMode;
  }
  if (slot.excludeFromPullContent) return "standalone";
  return "dashboard";
}

/**
 * Host 首页左卡占用：情景/会话 dashboard 面板 > 岛栏常驻（且为 dashboard）>
 * 否则 null（UI 再回落到正在播放 / 首个面板）。
 */
export function resolveHomeDashboardLeftPluginId(opts: {
  forcedDashboardPluginId: string | null | undefined;
  barResidentId: string | null | undefined;
  /** 常驻插件的 pullMode；非 dashboard / 未启用时传 null */
  barResidentPullMode: PanelPullMode | null | undefined;
}): string | null {
  const forced = String(opts.forcedDashboardPluginId ?? "").trim();
  if (forced) return forced;
  const resident = String(opts.barResidentId ?? "").trim();
  if (!resident) return null;
  if (opts.barResidentPullMode === "dashboard") return resident;
  return null;
}

/**
 * Plugins eligible for Host home left-card rail (caller may further filter surfaces).
 * Includes scenario plugins when `pullMode: dashboard`.
 * Dedupes `id` / `id__dev` — prefer non-dev.
 */
export function listDashboardPanelProviders(
  panelPlugins: PluginManifest[],
): DashboardPanelProvider[] {
  const mapped = panelPlugins
    .filter(
      (m) =>
        m.slots?.["island.panel"] &&
        (m.entry?.panel || m.entry?.development?.panel) &&
        resolvePanelPullMode(m) === "dashboard",
    )
    .map((m) => {
      const scenarioOrder = m.slots?.["island.scenario"]?.order;
      const barOrder = m.slots?.["island.bar"]?.order;
      return {
        id: m.id,
        label: m.name,
        description: m.description ?? "插件面板",
        order: scenarioOrder ?? barOrder ?? 100,
        manifest: m,
      };
    });

  const byBase = new Map<string, DashboardPanelProvider>();
  for (const p of mapped) {
    const base = p.id.replace(/__dev$/, "");
    const prev = byBase.get(base);
    if (!prev) {
      byBase.set(base, p);
      continue;
    }
    // Prefer non-dev over __dev duplicate
    const prevDev = prev.id.endsWith("__dev");
    const curDev = p.id.endsWith("__dev");
    if (prevDev && !curDev) byBase.set(base, p);
  }

  return [...byBase.values()].sort(
    (a, b) => a.order - b.order || a.label.localeCompare(b.label, "zh"),
  );
}
