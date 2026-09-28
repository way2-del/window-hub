/**
 * Shortcuts ⋯ fold list — carried into status-menu-popup HWND so the flyout
 * uses the same DWM / glass shell as status / tray menus (not an in-bar portal).
 */

export type ShortcutsFoldMenuItem = {
  pluginId: string;
  label: string;
};

export const SHORTCUTS_FOLD_PICK_EVENT = "shortcuts-fold-pick";
/** Fold-menu drag reorder → host persists pluginOrder. */
export const SHORTCUTS_FOLD_REORDER_EVENT = "shortcuts-fold-reorder";

export type ShortcutsFoldReorderPayload = {
  /** Wing that opened the fold menu. */
  side: "left" | "right";
  /** New overflow order within that wing (in-list reorder). */
  orderedIds: string[];
  /** Cross-wing move: plugin leaves `side` and lands on `moveToSide`. */
  movePluginId?: string;
  moveToSide?: "left" | "right";
};

let items: ShortcutsFoldMenuItem[] = [];
const listeners = new Set<() => void>();

export function getShortcutsFoldMenuItems(): ShortcutsFoldMenuItem[] {
  return items;
}

export function setShortcutsFoldMenuItems(next: ShortcutsFoldMenuItem[]): void {
  items = next.filter((it) => it.pluginId && it.label);
  for (const cb of listeners) cb();
}

export function clearShortcutsFoldMenuItems(): void {
  if (items.length === 0) return;
  items = [];
  for (const cb of listeners) cb();
}

export function subscribeShortcutsFoldMenu(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
