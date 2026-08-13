/** Auto plate color from icon PNG — same sampler as island notify stroke. */

import { dominantColorFromPngBase64 } from "./iconDominantColor";
import { DOCK_AUTO_PLATE_BG } from "./dockIcons";

const colorCache = new Map<string, string>();

/** `rgb(r, g, b)` → `#rrggbb` */
export function rgbCssToHex(rgb: string): string | null {
  const m = rgb.trim().match(/^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i);
  if (!m) return null;
  const h = (n: number) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return `#${h(Number(m[1]))}${h(Number(m[2]))}${h(Number(m[3]))}`;
}

/**
 * Dominant opaque brand color from a dock icon PNG (base64).
 * Skips near-black / near-white chrome — same pipeline as island notify stroke.
 */
export function plateColorFromPngBase64(b64: string): Promise<string> {
  const key = (b64 || "").trim();
  if (!key) return Promise.resolve(DOCK_AUTO_PLATE_BG);
  const hit = colorCache.get(key);
  if (hit) return Promise.resolve(hit);

  return dominantColorFromPngBase64(key).then((dbg) => {
    const hex = rgbCssToHex(dbg.color) || DOCK_AUTO_PLATE_BG;
    colorCache.set(key, hex);
    return hex;
  });
}

export function peekCachedPlateColor(b64: string | null | undefined): string | null {
  if (!b64) return null;
  return colorCache.get(b64.trim()) ?? null;
}
