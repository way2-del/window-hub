import { getCurrentWindow } from "@tauri-apps/api/window";

export type GenieAnchor = { x: number; y: number; w: number; h: number };

/** Screen logical rect for a DOM element (same convention as popup anchors). */
export async function elementScreenRect(el: HTMLElement): Promise<GenieAnchor> {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const r = el.getBoundingClientRect();
  return {
    x: outer.x / factor + r.left,
    y: outer.y / factor + r.top,
    w: Math.max(8, r.width),
    h: Math.max(8, r.height),
  };
}
