import type { Rgb } from "./tokens";

/** 从色带 PNG 左 / 中 / 右采样，左右与岛中文字对比度各用一端 */
export async function sampleStripBands(
  b64: string,
): Promise<{ left: Rgb; center: Rgb; right: Rgb } | null> {
  try {
    const img = new Image();
    img.decoding = "async";
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("png"));
      img.src = `data:image/png;base64,${b64}`;
    });
    const w = Math.max(1, img.naturalWidth);
    const h = Math.max(1, img.naturalHeight);
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0);
    const band = Math.max(1, Math.floor(w * 0.08));
    const avg = (x0: number, x1: number): Rgb => {
      const data = ctx.getImageData(x0, 0, Math.max(1, x1 - x0), h).data;
      let r = 0;
      let g = 0;
      let b = 0;
      let n = 0;
      for (let i = 0; i < data.length; i += 4) {
        r += data[i]!;
        g += data[i + 1]!;
        b += data[i + 2]!;
        n += 1;
      }
      return {
        r: Math.round(r / n),
        g: Math.round(g / n),
        b: Math.round(b / n),
      };
    };
    const mid0 = Math.max(0, Math.floor(w / 2 - band / 2));
    return {
      left: avg(0, band),
      center: avg(mid0, mid0 + band),
      right: avg(Math.max(0, w - band), w),
    };
  } catch {
    return null;
  }
}

