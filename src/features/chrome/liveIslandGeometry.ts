/**
 * Live island width while paintDom morphs (React `size` lags).
 * Shortcuts / tray fold subscribe here so chrome yields as the capsule grows.
 */

let liveIslandW = 300;
const listeners = new Set<(w: number) => void>();

export function getLiveIslandWidth(): number {
  return liveIslandW;
}

export function setLiveIslandWidth(width: number): void {
  const w = Math.max(28, Math.round(width));
  if (Math.abs(liveIslandW - w) < 1) return;
  liveIslandW = w;
  for (const cb of listeners) cb(w);
}

export function subscribeLiveIslandWidth(cb: (w: number) => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
