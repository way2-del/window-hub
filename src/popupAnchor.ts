/** Edge-aware popup anchor under a trigger element (logical screen coords). */

import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { placePopupNearRect } from "./features/chrome/popupPlacement";

const DEFAULT_GAP = 8;
/** Matches Rust PLUGIN_POPUP_W / H defaults. */
export const DEFAULT_PLUGIN_POPUP_W = 320;
export const DEFAULT_PLUGIN_POPUP_H = 480;

/**
 * Place a popup below `el`, right-aligning / flipping when near screen edges.
 * Rust `fit_popup_xy` remains the final safety net with real size.
 */
export async function anchorPopupBelowElement(
  el: HTMLElement,
  popupW: number = DEFAULT_PLUGIN_POPUP_W,
  popupH: number = DEFAULT_PLUGIN_POPUP_H,
  gap: number = DEFAULT_GAP,
): Promise<{ x: number; y: number }> {
  const win = getCurrentWindow();
  const [factor, outer, monitor] = await Promise.all([
    win.scaleFactor(),
    win.outerPosition(),
    currentMonitor(),
  ]);
  const rect = el.getBoundingClientRect();
  const ox = outer.x / factor;
  const oy = outer.y / factor;
  const trigger = {
    left: ox + rect.left,
    top: oy + rect.top,
    right: ox + rect.right,
    bottom: oy + rect.bottom,
  };

  let work = {
    left: 0,
    top: 0,
    right: window.screen.availWidth || 1920,
    bottom: window.screen.availHeight || 1080,
  };
  if (monitor) {
    const s = monitor.scaleFactor;
    const pos = monitor.position;
    const size = monitor.size;
    work = {
      left: pos.x / s,
      top: pos.y / s,
      right: pos.x / s + size.width / s,
      bottom: pos.y / s + size.height / s,
    };
  }

  return placePopupNearRect(trigger, popupW, popupH, work, gap);
}
