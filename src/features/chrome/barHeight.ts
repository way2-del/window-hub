/** Pure helpers: menubar / island strip height + chrome text scale. */

export const DEFAULT_BAR_H = 28;
export const MIN_BAR_H = 24;
export const MAX_BAR_H = 40;
export const BAR_H_STEP = 2;

/** Clamp + snap to step; invalid → default. */
export function normalizeBarHeight(raw: unknown): number {
  const n = typeof raw === "number" ? raw : Number(raw);
  if (!Number.isFinite(n)) return DEFAULT_BAR_H;
  const clamped = Math.min(MAX_BAR_H, Math.max(MIN_BAR_H, Math.round(n)));
  const steps = Math.round((clamped - MIN_BAR_H) / BAR_H_STEP);
  const snapped = MIN_BAR_H + steps * BAR_H_STEP;
  return Math.min(MAX_BAR_H, Math.max(MIN_BAR_H, snapped));
}

/** Scale vs default 28px bar (1 = stock size). */
export function chromeScale(barH: number): number {
  return normalizeBarHeight(barH) / DEFAULT_BAR_H;
}

let liveBarH = DEFAULT_BAR_H;

/** Runtime strip height after chrome prefs hydrate. */
export function getLiveBarHeight(): number {
  return liveBarH;
}

export function setLiveBarHeight(raw: unknown): number {
  liveBarH = normalizeBarHeight(raw);
  return liveBarH;
}

/** Write --island-bar-h / --chrome-scale on documentElement (or given root). */
export function applyBarHeightCss(
  barH: number = liveBarH,
  root: HTMLElement | null =
    typeof document !== "undefined" ? document.documentElement : null,
): void {
  if (!root) return;
  const h = normalizeBarHeight(barH);
  root.style.setProperty("--island-bar-h", `${h}px`);
  root.style.setProperty("--chrome-scale", String(chromeScale(h)));
}
