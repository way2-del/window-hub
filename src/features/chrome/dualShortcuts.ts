/** Pure policy: right-rail chrome tiers + dual / hybrid / tray shortcuts. */

export type ChromeVisibility = {
  showTray: boolean;
  showWifi: boolean;
  showClock: boolean;
  showIme: boolean;
  showControlCenter: boolean;
};

/** Tier0 dual | Tier1 hybrid (chips+shortcuts) | Tier2 full tray. */
export type ChromeRailTier = "dual" | "hybrid" | "tray";

export type DualShortcutsTransition = "enter" | "exit" | "none";

/** System chips (not the tray-icons rail). */
export function hasSystemChromeChips(prefs: ChromeVisibility): boolean {
  return (
    prefs.showWifi ||
    prefs.showClock ||
    prefs.showIme ||
    prefs.showControlCenter
  );
}

/**
 * - dual: all modules off → left+right shortcuts, no tray cluster
 * - hybrid: tray icons off, ≥1 system chip → far-right chips + inner right shortcuts
 * - tray: tray icons on → full tray, no right shortcuts
 */
export function resolveChromeRailTier(prefs: ChromeVisibility): ChromeRailTier {
  if (prefs.showTray) return "tray";
  if (hasSystemChromeChips(prefs)) return "hybrid";
  return "dual";
}

/** Right shortcuts wing exists in dual and hybrid. */
export function hasRightShortcutsWing(prefs: ChromeVisibility): boolean {
  return resolveChromeRailTier(prefs) !== "tray";
}

/** @deprecated prefer resolveChromeRailTier === "dual" */
export function isDualShortcutsMode(prefs: ChromeVisibility): boolean {
  return resolveChromeRailTier(prefs) === "dual";
}

export function dualShortcutsTransition(
  prev: ChromeVisibility,
  next: ChromeVisibility,
): DualShortcutsTransition {
  const was = isDualShortcutsMode(prev);
  const now = isDualShortcutsMode(next);
  if (!was && now) return "enter";
  if (was && !now) return "exit";
  return "none";
}

/** True when entering full-tray tier (right shortcuts must yield). */
export function enteredTrayTier(
  prev: ChromeVisibility,
  next: ChromeVisibility,
): boolean {
  return resolveChromeRailTier(prev) !== "tray" && resolveChromeRailTier(next) === "tray";
}

/** How many right-rail modules are still on. */
export function countRightChromeModules(prefs: ChromeVisibility): number {
  let n = 0;
  if (prefs.showTray) n += 1;
  if (prefs.showWifi) n += 1;
  if (prefs.showClock) n += 1;
  if (prefs.showIme) n += 1;
  if (prefs.showControlCenter) n += 1;
  return n;
}
