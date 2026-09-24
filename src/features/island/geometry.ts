import { clamp01 } from "./motion";

export const ISLAND_CORNER_PATCH_SIZE = 8;

// Inject the panel threshold from the owner; do not import persisted preferences here.
export function createIslandGeometry(STAGING_PANEL_H_DEFAULT: number) {
  /** 岛底圆角半径（与 islandPath 共用，供 BorderBeam 贴合） */
  function islandBottomRadius(width: number, height: number): number {
    const w = Math.max(28, width);
    const h = Math.max(28, height);
    const raw = Math.min(
      h * 0.5 - 0.01,
      Math.max(14, 14 + ((h - 28) * 18) / 192),
      w * 0.5 - 4,
    );
    // 较矮面板：底角过大时会切掉四角内容
    if (h <= STAGING_PANEL_H_DEFAULT + 4) return Math.min(raw, 18);
    return Math.min(raw, 32);
  }

  /**
   * 灵动岛路径（本地坐标：左上为 0,0，宽高=当前岛尺寸）。
   * 禁止再嵌进更大的「画布居中」坐标系，否则折叠宽与展开画布不一致时黑壳会偏/歪。
   * topSquare≥1：顶角真直角贴边；底角始终圆角。
   */
  function islandPath(width: number, height: number, topSquare = 0, topBleed = 0): string {
    const w = Math.max(28, width);
    const h = Math.max(28, height);
    const x0 = 0;
    const x1 = w;

    const rBot = islandBottomRadius(w, h);
    const flat = clamp01(topSquare);
    const squareTop = flat >= 0.999;
    const rTop = squareTop ? 0 : Math.max(0.05, rBot * (1 - flat));
    const k = 0.5522847498;
    const rkBot = rBot * k;
    // 顶边可上溢 topBleed，消除贴屏发丝缝；底边仍落在 h
    const y0 = -Math.max(0, topBleed);
    const y1 = h;
    const sideBot = y1 - rBot;

    if (squareTop) {
      // 顶边直角：纯直线拐角，不用贝塞尔
      return [
        `M ${fmt(x0)} ${fmt(y0)}`,
        `L ${fmt(x1)} ${fmt(y0)}`,
        `L ${fmt(x1)} ${fmt(sideBot)}`,
        `C ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1 - rBot)} ${fmt(y1)}`,
        `L ${fmt(x0 + rBot)} ${fmt(y1)}`,
        `C ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0)} ${fmt(sideBot)}`,
        `L ${fmt(x0)} ${fmt(y0)}`,
        `Z`,
      ].join(" ");
    }

    const rkTop = rTop * k;
    const sideTop = 0 + rTop;
    return [
      `M ${fmt(x0 + rTop)} ${fmt(y0)}`,
      `L ${fmt(x1 - rTop)} ${fmt(y0)}`,
      `C ${fmt(x1 - rTop + rkTop)} ${fmt(0)}, ${fmt(x1)} ${fmt(0 + rTop - rkTop)}, ${fmt(x1)} ${fmt(sideTop)}`,
      `L ${fmt(x1)} ${fmt(sideBot)}`,
      `C ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1 - rBot)} ${fmt(y1)}`,
      `L ${fmt(x0 + rBot)} ${fmt(y1)}`,
      `C ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0)} ${fmt(sideBot)}`,
      `L ${fmt(x0)} ${fmt(sideTop)}`,
      `C ${fmt(x0)} ${fmt(0 + rTop - rkTop)}, ${fmt(x0 + rTop - rkTop)} ${fmt(0)}, ${fmt(x0 + rTop)} ${fmt(0)}`,
      `Z`,
    ].join(" ");
  }

  /**
   * 通知描边开口路径：顶左右沿补丁凹弧贴合（凹进去，非外凸耳朵）；不含顶边。
   * 凹弧圆心在补丁外角 ( ±p, p )，从顶外尖接到岛侧壁。
   */
  function islandNotifyInnerStrokePath(
    width: number,
    height: number,
    patch = ISLAND_CORNER_PATCH_SIZE,
  ): string {
    const w = Math.max(28, width);
    const h = Math.max(28, height);
    const p = Math.max(4, patch);
    const x0 = 0;
    const x1 = w;
    const rBot = islandBottomRadius(w, h);
    const k = 0.5522847498;
    const rkBot = rBot * k;
    const y1 = h;
    const sideBot = y1 - rBot;
    // 凹弧控制点：圆心在 (±p, p)
    const p1k = p * (1 - k);

    return [
      // 左：顶外尖 (-p,0) → 凹弧 → 岛左壁 (0,p)
      `M ${fmt(-p)} ${fmt(0)}`,
      `C ${fmt(-p1k)} ${fmt(0)}, ${fmt(x0)} ${fmt(p1k)}, ${fmt(x0)} ${fmt(p)}`,
      `L ${fmt(x0)} ${fmt(sideBot)}`,
      `C ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0 + rBot)} ${fmt(y1)}`,
      `L ${fmt(x1 - rBot)} ${fmt(y1)}`,
      `C ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1)} ${fmt(sideBot)}`,
      `L ${fmt(x1)} ${fmt(p)}`,
      // 右：岛右壁 (w,p) → 凹弧 → 顶外尖 (w+p,0)
      `C ${fmt(x1)} ${fmt(p1k)}, ${fmt(x1 + p1k)} ${fmt(0)}, ${fmt(x1 + p)} ${fmt(0)}`,
    ].join(" ");
  }

  /** 通知描边 clip：岛身 + 左右凹角补丁（闭合） */
  function islandNotifyClipSilhouette(
    width: number,
    height: number,
    patch = ISLAND_CORNER_PATCH_SIZE,
    topBleed = 0,
  ): string {
    const w = Math.max(28, width);
    const h = Math.max(28, height);
    const p = Math.max(4, patch);
    const bleed = Math.max(0, topBleed);
    const x0 = 0;
    const x1 = w;
    const rBot = islandBottomRadius(w, h);
    const k = 0.5522847498;
    const rkBot = rBot * k;
    const p1k = p * (1 - k);
    const yTop = -bleed;
    const y1 = h;
    const sideBot = y1 - rBot;

    return [
      `M ${fmt(-p)} ${fmt(yTop)}`,
      `L ${fmt(x1 + p)} ${fmt(yTop)}`,
      `L ${fmt(x1 + p)} ${fmt(0)}`,
      // 右凹弧：外尖 → 岛右壁
      `C ${fmt(x1 + p1k)} ${fmt(0)}, ${fmt(x1)} ${fmt(p1k)}, ${fmt(x1)} ${fmt(p)}`,
      `L ${fmt(x1)} ${fmt(sideBot)}`,
      `C ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1 - rBot)} ${fmt(y1)}`,
      `L ${fmt(x0 + rBot)} ${fmt(y1)}`,
      `C ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0)} ${fmt(sideBot)}`,
      `L ${fmt(x0)} ${fmt(p)}`,
      // 左凹弧：岛左壁 → 外尖
      `C ${fmt(x0)} ${fmt(p1k)}, ${fmt(-p1k)} ${fmt(0)}, ${fmt(-p)} ${fmt(0)}`,
      `L ${fmt(-p)} ${fmt(yTop)}`,
      `Z`,
    ].join(" ");
  }

  function fmt(n: number) {
    return (Math.round(n * 10) / 10).toString();
  }

  return { islandBottomRadius, islandPath, islandNotifyInnerStrokePath, islandNotifyClipSilhouette };
}
