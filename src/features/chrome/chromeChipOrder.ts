/** Pure helpers: right-rail system chip order (wifi / ime / controlCenter / clock). */

export const CHROME_CHIP_IDS = [
  "wifi",
  "ime",
  "controlCenter",
  "clock",
] as const;

export type ChromeChipId = (typeof CHROME_CHIP_IDS)[number];

export const DEFAULT_CHROME_CHIP_ORDER: ChromeChipId[] = [
  "wifi",
  "ime",
  "controlCenter",
  "clock",
];

export function isChromeChipId(id: string): id is ChromeChipId {
  return (CHROME_CHIP_IDS as readonly string[]).includes(id);
}

/** Normalize persisted order: known ids once, append missing in default order. */
export function normalizeChromeChipOrder(
  raw: string[] | null | undefined,
): ChromeChipId[] {
  const seen = new Set<string>();
  const out: ChromeChipId[] = [];
  for (const id of raw ?? []) {
    if (!isChromeChipId(id) || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  for (const id of DEFAULT_CHROME_CHIP_ORDER) {
    if (seen.has(id)) continue;
    out.push(id);
  }
  return out;
}

/** Visible chips in display order (respect prefs + order). */
export function orderedVisibleChromeChips(
  order: string[] | null | undefined,
  visible: Partial<Record<ChromeChipId, boolean>>,
): ChromeChipId[] {
  return normalizeChromeChipOrder(order).filter((id) => visible[id] !== false);
}
