/**
 * 灵动岛下拉面板几何常量（与 .cursor/skills/window-hub-island-panel 契约一致）。
 * 默认天气/镜子硬顶；中转站矮宽见 islandPrefs STAGING_PANEL_*。
 */
import { DEFAULT_BAR_H } from "../features/chrome/barHeight";

export const ISLAND_VIEW_W = 380;
export const ISLAND_VIEW_H = 220;
/** 默认折叠岛高；运行时用 getLiveBarHeight() / --island-bar-h。 */
export const ISLAND_BAR_H = DEFAULT_BAR_H;

/** 展开态岛体外轮廓底角（SVG islandPath rBot） */
export const ISLAND_SHELL_RADIUS = 32;

/**
 * 下拉内容区内边距：左 = 右 = 底；顶略小以贴近天气条。
 * 禁止底边距大于左右（曾用 max-height 留白造成「底边更厚」）。
 */
export const ISLAND_PANEL_INSET = 16;
export const ISLAND_PANEL_INSET_TOP = 10;

/** 面板内媒体/卡片圆角：与岛壳底角同值 */
export const ISLAND_PANEL_RADIUS = ISLAND_SHELL_RADIUS;
