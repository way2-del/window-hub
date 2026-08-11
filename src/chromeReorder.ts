/** Shared helpers for Ctrl+drag layout order (shortcuts + tray). */

export function sortByOrderKey<T>(
  items: T[],
  order: string[],
  idOf: (item: T) => string,
): T[] {
  if (!order.length || items.length <= 1) return items;
  const rank = new Map(order.map((id, i) => [id, i]));
  return [...items].sort((a, b) => {
    const ia = rank.get(idOf(a));
    const ib = rank.get(idOf(b));
    if (ia == null && ib == null) return 0;
    if (ia == null) return 1;
    if (ib == null) return -1;
    return ia - ib;
  });
}

/** Move `fromId` to before/after `toId` within `order` (ids not in order are appended). */
export function moveIdInOrder(
  order: string[],
  fromId: string,
  toId: string,
  place: "before" | "after",
): string[] {
  if (!fromId || !toId || fromId === toId) return order;
  const base = order.filter((id) => id !== fromId);
  const toIdx = base.indexOf(toId);
  if (toIdx < 0) {
    return [...base, fromId];
  }
  const insertAt = place === "before" ? toIdx : toIdx + 1;
  const next = [...base];
  next.splice(insertAt, 0, fromId);
  return next;
}

/** Given pointer X and a list of element rects (in order), pick drop target id + place. */
export function pickDropTarget(
  clientX: number,
  units: Array<{ id: string; left: number; width: number }>,
  dragId: string,
): { toId: string; place: "before" | "after" } | null {
  const others = units.filter((u) => u.id !== dragId);
  if (!others.length) return null;
  for (const u of others) {
    const mid = u.left + u.width / 2;
    if (clientX < mid) return { toId: u.id, place: "before" };
  }
  const last = others[others.length - 1]!;
  return { toId: last.id, place: "after" };
}

export function sameOrder(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((id, i) => id === b[i]);
}
