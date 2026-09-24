// Pure interpolation and gesture progress; no UI/window side effects.
export function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t;
}

export function clamp01(t: number) {
  return Math.max(0, Math.min(1, t));
}

/** 下拉进度：原位点不动，弹窗高度随拖拽增大 */
export function pullProgress(dy: number) {
  if (dy <= 0) return 0;
  const t = clamp01(dy / 210);
  // 阻力：越拉越沉
  return 1 - Math.pow(1 - t, 1.85);
}

/** 近似 cubic-bezier(0.22, 1, 0.36, 1)：快起、尾段丝滑 */
export function easeOutSmooth(t: number) {
  const x = clamp01(t);
  // 用 1-(1-x)^3 与 softer 混合，避免「砸到位」的顿挫
  const a = 1 - Math.pow(1 - x, 3);
  const b = x * x * (3 - 2 * x); // smoothstep
  return a * 0.72 + b * 0.28;
}

/** 把全局进度映射到 [start,end] 子区间，再 ease */
export function channelEase(p: number, start: number, end: number) {
  return easeOutSmooth(clamp01((p - start) / Math.max(0.001, end - start)));
}
