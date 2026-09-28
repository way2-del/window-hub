/**
 * Ask the main island to collapse when chrome fold / tray would otherwise
 * open a flyout on top of an expanded search/panel island.
 */
import { emit } from "@tauri-apps/api/event";

export const ISLAND_REQUEST_COLLAPSE_EVENT = "island-request-collapse";

/**
 * Island is occupying chrome space (panel / search / pull) — fold & tray
 * flyouts should collapse it first instead of stacking on top.
 */
export function isIslandShellExpanded(): boolean {
  if (typeof document === "undefined") return false;
  const root = document.querySelector(".island-root");
  if (!root) return false;
  if (
    document.querySelector(".shell.is-expanded") ||
    root.classList.contains("is-expanded") ||
    root.classList.contains("is-searching") ||
    root.classList.contains("is-pulling") ||
    root.classList.contains("is-springing")
  ) {
    return true;
  }
  const beam = document.querySelector(".island-beam") as HTMLElement | null;
  if (!beam) return false;
  // Tall panel / pull reveal still squeezes chrome.
  return beam.getBoundingClientRect().height > 36;
}

/** @returns true if a collapse was requested (caller should skip opening menus). */
export async function requestIslandCollapseIfExpanded(): Promise<boolean> {
  if (!isIslandShellExpanded()) return false;
  await emit(ISLAND_REQUEST_COLLAPSE_EVENT).catch(() => undefined);
  return true;
}
