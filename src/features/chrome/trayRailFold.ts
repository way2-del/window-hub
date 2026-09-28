/**
 * Pure tray-rail fold: when the island widens, pin icons that no longer fit
 * into the chevron popup until the island shrinks again.
 */

/** Keep in sync with shortcutsGeometry SHORTCUTS_* (chrome feature must not import plugins/). */
const ISLAND_GAP_MIN = 12;
/**
 * Snug gap island↔tray: tighter than side-fade width so icons hug the capsule;
 * the fade soft-veils the last few px instead of clearing the whole rail.
 */
export const TRAY_ISLAND_SNUG_GAP = 20;
const RIGHT_INSET = 10;

/** 16px glyph + 7px rail gap (matches .tray-rail / .tray-icon-btn). */
export const TRAY_ICON_SLOT_W = 23;

/**
 * Max width available for the whole tray rail (icons + chips + chevron)
 * between island-right+gap and the screen right inset.
 */
export function computeTrayRailMaxWidth(
  viewportW: number,
  islandW: number,
  islandGap: number = TRAY_ISLAND_SNUG_GAP,
  rightInset: number = RIGHT_INSET,
): number {
  const gap = Math.max(ISLAND_GAP_MIN, islandGap);
  const vw = Math.max(0, viewportW);
  const iw = Math.max(0, islandW);
  const islandRight = vw / 2 + iw / 2;
  return Math.max(0, vw - rightInset - (islandRight + gap));
}

/** Width left for tray icons after reserving system chips + chevron. */
export function computeTrayIconBudget(
  maxRailW: number,
  fixedChromeW: number,
): number {
  return Math.max(0, Math.max(0, maxRailW) - Math.max(0, fixedChromeW));
}

export type TrayIconFoldPlan = {
  visibleIds: string[];
  overflowIds: string[];
};

/**
 * Pack rail icons into `iconBudget`. Prefer icons nearest the screen edge
 * (end of `orderedIds`); island-facing icons overflow into the chevron first.
 * Partial slot (≥35%) still counts — sit snug under the side fade.
 */
export function planTrayIconFold(
  orderedIds: string[],
  iconBudget: number,
  slotW: number = TRAY_ICON_SLOT_W,
): TrayIconFoldPlan {
  const ids = orderedIds.filter(Boolean);
  if (ids.length === 0) return { visibleIds: [], overflowIds: [] };
  const slot = Math.max(1, Math.round(slotW));
  const budget = Math.max(
    0,
    Number.isFinite(iconBudget) ? Math.max(0, iconBudget) : 0,
  );
  const maxCount = Math.floor((budget + slot * 0.35) / slot);
  if (maxCount >= ids.length) {
    return { visibleIds: ids, overflowIds: [] };
  }
  if (maxCount <= 0) {
    return { visibleIds: [], overflowIds: ids };
  }
  const visibleIds = ids.slice(ids.length - maxCount);
  const overflowIds = ids.slice(0, ids.length - maxCount);
  return { visibleIds, overflowIds };
}
