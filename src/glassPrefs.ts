/** Shared glass panel opacity for settings / tray / plugin popups. */

import { invoke } from "@tauri-apps/api/core";

export type GlassKind = "mica-alt" | "blur" | "aero" | "acrylic";

export type GlassPrefs = {
  kind: GlassKind | string;
  /** null/undefined = follow system (resolved to concrete dark/light at apply time) */
  dark?: boolean | null;
  acrylicAlpha?: number;
};

/** Temporarily always MicaAlt. */
export function normalizeGlassKind(_k?: string | null): GlassKind {
  return "mica-alt";
}

/** System app theme via CSS media (fallback when Rust command unavailable). */
export function systemPrefersDark(): boolean {
  if (typeof window !== "undefined" && window.matchMedia) {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  }
  return true;
}

/**
 * Resolve auto (`null`) to concrete dark/light — never a third hybrid look.
 * Prefer an explicit `resolved` from Rust (`system_apps_dark`) when provided.
 */
export function resolveDark(prefs: GlassPrefs, resolvedSystemDark?: boolean): boolean {
  if (prefs.dark === true) return true;
  if (prefs.dark === false) return false;
  if (typeof resolvedSystemDark === "boolean") return resolvedSystemDark;
  return systemPrefersDark();
}

/**
 * System Mica wash + light/dark UI tokens.
 * Keep washes thin so SYSTEMBACKDROP mica actually shows through (Start-menu-like).
 */
export function applyGlassCss(prefs: GlassPrefs, resolvedSystemDark?: boolean) {
  const dark = resolveDark(prefs, resolvedSystemDark);
  const root = document.documentElement;
  root.dataset.glass = "mica";
  root.dataset.theme = dark ? "dark" : "light";
  root.style.colorScheme = dark ? "dark" : "light";

  if (dark) {
    root.style.setProperty("--glass-panel-bg", "rgba(28, 28, 30, 0.48)");
    root.style.setProperty("--glass-main-bg", "#1c1c1e");
    root.style.setProperty("--glass-fg", "#f4f4f5");
    root.style.setProperty("--glass-fg-muted", "#a1a1aa");
    root.style.setProperty("--glass-side", "transparent");
    root.style.setProperty("--glass-card", "rgba(255, 255, 255, 0.07)");
    root.style.setProperty("--glass-border", "rgba(255, 255, 255, 0.1)");
    root.style.setProperty("--glass-input", "rgba(0, 0, 0, 0.22)");
    root.style.setProperty("--glass-btn", "rgba(255, 255, 255, 0.08)");
    root.style.setProperty("--glass-btn-border", "rgba(255, 255, 255, 0.12)");
  } else {
    root.style.setProperty("--glass-panel-bg", "rgba(255, 255, 255, 0.52)");
    root.style.setProperty("--glass-main-bg", "#f2f2f7");
    root.style.setProperty("--glass-fg", "#1c1c1e");
    root.style.setProperty("--glass-fg-muted", "#3f3f46");
    root.style.setProperty("--glass-side", "transparent");
    root.style.setProperty("--glass-card", "rgba(255, 255, 255, 0.85)");
    root.style.setProperty("--glass-border", "rgba(0, 0, 0, 0.1)");
    root.style.setProperty("--glass-input", "rgba(255, 255, 255, 0.72)");
    root.style.setProperty("--glass-btn", "rgba(255, 255, 255, 0.9)");
    root.style.setProperty("--glass-btn-border", "rgba(0, 0, 0, 0.14)");
  }
}

/** Apply CSS using the same Windows Apps theme Rust uses for Mica immersive mode. */
export async function syncGlassCss(prefs: GlassPrefs): Promise<boolean> {
  let systemDark: boolean | undefined;
  if (prefs.dark == null) {
    try {
      systemDark = await invoke<boolean>("system_apps_dark");
    } catch {
      /* matchMedia fallback */
    }
  }
  applyGlassCss(prefs, systemDark);
  return resolveDark(prefs, systemDark);
}

/** Re-run when OS app theme flips while preference is “跟随系统”. */
export function subscribeSystemDark(onChange: (dark: boolean) => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => undefined;
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = () => onChange(mq.matches);
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}
