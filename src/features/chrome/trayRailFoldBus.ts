/**
 * Island-squeeze tray overflow IDs (temporary — restore when island shrinks).
 * TrayCluster publishes; TrayPopupApp reads so folded icons appear under 已收纳.
 */

export type TrayRailFoldPayload = {
  overflowIds: string[];
};

let last: TrayRailFoldPayload = { overflowIds: [] };
const listeners = new Set<(p: TrayRailFoldPayload) => void>();

export function getTrayRailFold(): TrayRailFoldPayload {
  return last;
}

export function setTrayRailFold(payload: TrayRailFoldPayload): void {
  const overflowIds = (payload.overflowIds ?? []).filter(Boolean);
  const prev = last.overflowIds;
  if (
    prev.length === overflowIds.length &&
    prev.every((id, i) => id === overflowIds[i])
  ) {
    return;
  }
  last = { overflowIds };
  for (const cb of listeners) cb(last);
}

export function subscribeTrayRailFold(
  cb: (p: TrayRailFoldPayload) => void,
): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
