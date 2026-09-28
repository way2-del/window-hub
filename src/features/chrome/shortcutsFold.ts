/** Pure fold policy: which shortcut strips stay visible vs overflow into ⋯. */

export const SHORTCUTS_FOLD_CHIP_W = 28;
/** Match .shortcuts-collapsed / .tray-rail gap. */
export const SHORTCUTS_FOLD_GAP = 7;

export type FoldPlan = {
  visibleIds: string[];
  overflowIds: string[];
};

/**
 * Pack plugins into `maxWidth`, reserving space for an overflow chip when needed.
 * ⋯ sits on the island-facing edge of each wing:
 * - left wing: keep start (screen edge), overflow toward island (end)
 * - right wing: keep end (tray edge), overflow toward island (start)
 * Soft mask fades the island-facing clipped edge into ⋯.
 */
export function planShortcutsFold(
  orderedIds: string[],
  widths: Record<string, number>,
  maxWidth: number,
  side: "left" | "right",
  overflowChipW: number = SHORTCUTS_FOLD_CHIP_W,
  gap: number = SHORTCUTS_FOLD_GAP,
): FoldPlan {
  const ids = orderedIds.filter(Boolean);
  if (ids.length === 0) return { visibleIds: [], overflowIds: [] };
  const budget = Math.max(0, maxWidth);
  const widthOf = (id: string) => Math.max(0, Math.round(widths[id] ?? 28));
  const gapW = Math.max(0, Math.round(gap));

  let total = 0;
  for (let i = 0; i < ids.length; i++) {
    total += widthOf(ids[i]!);
    if (i > 0) total += gapW;
  }
  if (total <= budget) {
    return { visibleIds: ids, overflowIds: [] };
  }

  const packBudget = Math.max(0, budget - overflowChipW - (overflowChipW > 0 ? gapW : 0));
  const visible: string[] = [];
  let used = 0;

  if (side === "left") {
    // Prefer plugins nearest the screen edge (start of order).
    for (let i = 0; i < ids.length; i++) {
      const id = ids[i]!;
      const w = widthOf(id);
      const next = used + w + (visible.length > 0 ? gapW : 0);
      if (next > packBudget) break;
      visible.push(id);
      used = next;
    }
  } else {
    // Prefer plugins nearest the tray / screen edge (end of order).
    for (let i = ids.length - 1; i >= 0; i--) {
      const id = ids[i]!;
      const w = widthOf(id);
      const next = used + w + (visible.length > 0 ? gapW : 0);
      if (next > packBudget) break;
      visible.unshift(id);
      used = next;
    }
  }
  const overflowIds = ids.filter((id) => !visible.includes(id));
  return { visibleIds: visible, overflowIds };
}
