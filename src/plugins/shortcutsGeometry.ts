/**
 * 快捷区几何常量（与 .cursor/skills/window-hub-shortcuts 契约一致）。
 * 基座 ShortcutsHost 实现时应引用本文件，避免魔法数漂移。
 */
import {
  DEFAULT_BAR_H,
  getLiveBarHeight,
} from "../features/chrome/barHeight";

export const SHORTCUTS_LEFT_INSET = 12;
/** 右侧快捷区距屏幕右缘（与 .tray-cluster right:10px 对齐） */
export const SHORTCUTS_RIGHT_INSET = 10;
/** 默认安全距；运行时不得低于 SHORTCUTS_ISLAND_GAP_MIN */
export const SHORTCUTS_ISLAND_GAP = 24;
export const SHORTCUTS_ISLAND_GAP_MIN = 16;
/** 岛左右材质渐变带宽度；chrome 折叠间隙至少让出此区，避免图标压在渐变下 */
export const ISLAND_SIDE_FADE_W = 56;
/** 岛↔快捷/托盘有效间隙（含侧渐变带） */
export const SHORTCUTS_ISLAND_CLEARANCE = Math.max(
  SHORTCUTS_ISLAND_GAP,
  ISLAND_SIDE_FADE_W,
);
/** 右侧快捷区与系统芯片条间距（与 .tray-rail gap 一致） */
export const SHORTCUTS_CHROME_STRIP_GAP = 7;
/**
 * 默认顶栏高度（逻辑 px）。运行时请用 `getLiveBarHeight()` /
 * `hub.shortcuts.getBounds().height` / CSS `--wh-bar-h`，勿假定恒为 28。
 */
export const SHORTCUTS_HEIGHT = DEFAULT_BAR_H;
/** 与 SHORTCUTS_HEIGHT 同值；文档/技能中称「状态栏高度」时用此别名。 */
export const STATUS_MENU_BAR_HEIGHT = SHORTCUTS_HEIGHT;
export const SHORTCUTS_CHIP_MAX_W = 120;
/** 固定到快捷区的 pin 默认/最小/最大宽度 */
export const SHORTCUTS_PIN_DEFAULT_W = 96;
export const SHORTCUTS_PIN_MIN_W = 56;
export const SHORTCUTS_PIN_MAX_W = 220;
export const SHORTCUTS_EXPAND_MAX_RATIO = 1;
export const SHORTCUTS_SCROLL_STEP = 100;

export type ShortcutsBounds = {
  x: number;
  width: number;
  height: number;
  maxExpandWidth: number;
};

/**
 * 计算快捷区可用宽（逻辑像素）。
 * @param settingsRight 设置按钮右缘 x
 * @param islandLeft 灵动岛左缘 x
 * @param islandGap 使用的 gap，默认 SHORTCUTS_ISLAND_GAP
 */
export function computeShortcutsBounds(
  settingsRight: number,
  islandLeft: number,
  islandGap: number = SHORTCUTS_ISLAND_CLEARANCE,
): ShortcutsBounds {
  const gap = Math.max(SHORTCUTS_ISLAND_GAP_MIN, islandGap);
  const x = settingsRight + SHORTCUTS_LEFT_INSET;
  const rightMax = islandLeft - gap;
  const maxExpandWidth = Math.max(0, rightMax - x);
  return {
    x,
    width: maxExpandWidth,
    height: getLiveBarHeight(),
    maxExpandWidth,
  };
}

/**
 * 灵动岛右侧快捷区可用带：岛右缘 + gap → 视口右缘 − inset − 系统芯片条（逻辑像素）。
 * Host 贴右缘（或芯片条左侧）定位，内容从右往左长；`x` 仅为带左界。
 * @param chromeStripW Tier1 最外缘系统芯片条宽度；0 = 无芯片条
 * @param chromeStripGap 快捷↔芯片条间距，默认 7（与托盘内 gap 一致）
 */
export function computeShortcutsBoundsRight(
  islandRight: number,
  viewportRight: number,
  islandGap: number = SHORTCUTS_ISLAND_CLEARANCE,
  chromeStripW: number = 0,
  chromeStripGap: number = SHORTCUTS_CHROME_STRIP_GAP,
): ShortcutsBounds {
  const gap = Math.max(SHORTCUTS_ISLAND_GAP_MIN, islandGap);
  const strip = Math.max(0, chromeStripW);
  const stripGap = strip > 0 ? Math.max(0, chromeStripGap) : 0;
  const x = islandRight + gap;
  const rightMax = viewportRight - SHORTCUTS_RIGHT_INSET - strip - stripGap;
  const maxExpandWidth = Math.max(0, rightMax - x);
  return {
    x,
    width: maxExpandWidth,
    height: getLiveBarHeight(),
    maxExpandWidth,
  };
}
