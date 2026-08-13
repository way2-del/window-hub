import { useEffect, useState } from "react";

const plateCache = new Map<string, boolean>();

/**
 * True when the raster is mostly transparent at the outer frame —
 * those icons need a solid plate behind them.
 */
export function iconNeedsSolidPlate(src: string): Promise<boolean> {
  const hit = plateCache.get(src);
  if (hit != null) return Promise.resolve(hit);

  return new Promise((resolve) => {
    const img = new Image();
    img.decoding = "async";
    img.onload = () => {
      try {
        const size = 48;
        const canvas = document.createElement("canvas");
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) {
          plateCache.set(src, true);
          resolve(true);
          return;
        }
        ctx.clearRect(0, 0, size, size);
        ctx.drawImage(img, 0, 0, size, size);
        const { data } = ctx.getImageData(0, 0, size, size);
        const border = Math.max(2, Math.floor(size * 0.12));
        let edge = 0;
        let edgeClear = 0;
        let opaque = 0;
        for (let y = 0; y < size; y++) {
          for (let x = 0; x < size; x++) {
            const a = data[(y * size + x) * 4 + 3];
            if (a > 200) opaque += 1;
            const onEdge = x < border || y < border || x >= size - border || y >= size - border;
            if (!onEdge) continue;
            edge += 1;
            if (a < 40) edgeClear += 1;
          }
        }
        // Mostly clear frame → no baked plate. Prefer solid plate when unsure.
        const edgeClearRatio = edge > 0 ? edgeClear / edge : 1;
        const opaqueRatio = opaque / (size * size);
        const needs = opaqueRatio < 0.12 || edgeClearRatio > 0.28;
        plateCache.set(src, needs);
        resolve(needs);
      } catch {
        plateCache.set(src, true);
        resolve(true);
      }
    };
    img.onerror = () => {
      plateCache.set(src, true);
      resolve(true);
    };
    img.src = src;
  });
}

/** Raster dock tile: solid plate when icon has no opaque background. */
export function useDockIconPlate(src: string | null | undefined): boolean {
  const [needs, setNeeds] = useState(() => {
    if (!src) return true;
    return plateCache.get(src) ?? true;
  });

  useEffect(() => {
    if (!src) {
      setNeeds(true);
      return;
    }
    const cached = plateCache.get(src);
    if (cached != null) {
      setNeeds(cached);
      return;
    }
    let alive = true;
    void iconNeedsSolidPlate(src).then((v) => {
      if (alive) setNeeds(v);
    });
    return () => {
      alive = false;
    };
  }, [src]);

  return needs;
}
