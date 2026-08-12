/** Sample a PNG (base64) and return the largest-area opaque color. */

export type DominantColorDebug = {
  color: string;
  /** Why / how we picked it */
  reason: "empty" | "decode-fail" | "no-ctx" | "no-opaque" | "ok";
  iconBytes: number;
  sampleW: number;
  sampleH: number;
  opaque: number;
  skippedChrome: number;
  buckets: number;
  top: Array<{ color: string; n: number }>;
};

const FALLBACK = "rgb(52, 199, 89)";

function quantize(n: number, step = 24): number {
  return Math.round(n / step) * step;
}

function toRgb(r: number, g: number, b: number): string {
  return `rgb(${r}, ${g}, ${b})`;
}

export function dominantColorFromPngBase64(b64: string): Promise<DominantColorDebug> {
  const raw = (b64 || "").trim();
  if (!raw) {
    return Promise.resolve({
      color: FALLBACK,
      reason: "empty",
      iconBytes: 0,
      sampleW: 0,
      sampleH: 0,
      opaque: 0,
      skippedChrome: 0,
      buckets: 0,
      top: [],
    });
  }

  return new Promise((resolve) => {
    const img = new Image();
    img.decoding = "async";
    img.onload = () => {
      try {
        const canvas = document.createElement("canvas");
        const maxSide = 32;
        const scale = Math.min(1, maxSide / Math.max(img.width || 1, img.height || 1));
        const w = Math.max(1, Math.round((img.width || 1) * scale));
        const h = Math.max(1, Math.round((img.height || 1) * scale));
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        if (!ctx) {
          resolve({
            color: FALLBACK,
            reason: "no-ctx",
            iconBytes: raw.length,
            sampleW: w,
            sampleH: h,
            opaque: 0,
            skippedChrome: 0,
            buckets: 0,
            top: [],
          });
          return;
        }
        ctx.clearRect(0, 0, w, h);
        ctx.drawImage(img, 0, 0, w, h);
        const { data } = ctx.getImageData(0, 0, w, h);
        const counts = new Map<string, { n: number; r: number; g: number; b: number }>();
        let opaque = 0;
        let skippedChrome = 0;
        for (let i = 0; i < data.length; i += 4) {
          if (data[i + 3]! < 128) continue;
          opaque += 1;
          const r = data[i]!;
          const g = data[i + 1]!;
          const b = data[i + 2]!;
          const maxc = Math.max(r, g, b);
          const minc = Math.min(r, g, b);
          // Skip near-black / near-white chrome so brand colors win.
          if (maxc < 28 || minc > 230) {
            skippedChrome += 1;
            continue;
          }
          const qr = quantize(r);
          const qg = quantize(g);
          const qb = quantize(b);
          const key = `${qr},${qg},${qb}`;
          const cur = counts.get(key);
          if (cur) cur.n += 1;
          else counts.set(key, { n: 1, r: qr, g: qg, b: qb });
        }
        const ranked = [...counts.values()].sort((a, b) => b.n - a.n);
        const top = ranked.slice(0, 5).map((v) => ({
          color: toRgb(v.r, v.g, v.b),
          n: v.n,
        }));
        if (ranked.length === 0) {
          resolve({
            color: FALLBACK,
            reason: "no-opaque",
            iconBytes: raw.length,
            sampleW: w,
            sampleH: h,
            opaque,
            skippedChrome,
            buckets: 0,
            top: [],
          });
          return;
        }
        const best = ranked[0]!;
        resolve({
          color: toRgb(best.r, best.g, best.b),
          reason: "ok",
          iconBytes: raw.length,
          sampleW: w,
          sampleH: h,
          opaque,
          skippedChrome,
          buckets: ranked.length,
          top,
        });
      } catch {
        resolve({
          color: FALLBACK,
          reason: "decode-fail",
          iconBytes: raw.length,
          sampleW: 0,
          sampleH: 0,
          opaque: 0,
          skippedChrome: 0,
          buckets: 0,
          top: [],
        });
      }
    };
    img.onerror = () =>
      resolve({
        color: FALLBACK,
        reason: "decode-fail",
        iconBytes: raw.length,
        sampleW: 0,
        sampleH: 0,
        opaque: 0,
        skippedChrome: 0,
        buckets: 0,
        top: [],
      });
    img.src = `data:image/png;base64,${raw}`;
  });
}
