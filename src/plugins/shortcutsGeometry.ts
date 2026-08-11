/**
 * 快捷区几何常量（与 .cursor/skills/window-hub-shortcuts 契约一致）。
 * 基座 ShortcutsHost 实现时应引用本文件，避免魔法数漂移。
 */
export const SHORTCUTS_LEFT_INSET = 8;
/** 默认安全距；运行时不得低于 SHORTCUTS_ISLAND_GAP_MIN */
export const SHORTCUTS_ISLAND_GAP = 24;
export const SHORTCUTS_ISLAND_GAP_MIN = 16;
/**
 * 状态菜单顶栏高度（逻辑 px）= 快捷区 iframe 高度 = `--island-bar-h`。
 * 插件经 `hub.shortcuts.getBounds().height` / CSS `--wh-bar-h` 读取，勿写死。
 */
export const SHORTCUTS_HEIGHT = 28;
/** 与 SHORTCUTS_HEIGHT 同值；文档/技能中称「状态栏高度」时用此别名。 */
export const STATUS_MENU_BAR_HEIGHT = SHORTCUTS_HEIGHT;
export const SHORTCUTS_CHIP_MAX_W = 120;
/** 固定到快捷区的 pin 默认/最小/最大宽度 */
export const SHORTCUTS_PIN_DEFAULT_W = 96;
export const SHORTCUTS_PIN_MIN_W = 56;
export const SHORTCUTS_PIN_MAX_W = 220;
export const SHORTCUTS_EXPAND_MAX_RATIO = 1;
export const SHORTCUTS_SCROLL_STEP = 100;

/**
 * 左侧快捷区不再悬停即开弹窗（一律点击 / 拖入）。
 * 保留函数以免旧调用方报错；恒为 false。
 */
export function shortcutsAllowsHoverOpen(_pluginId: string): boolean {
  return false;
}

/** 拖入快捷区条时自动开表面：仅中转站（接文件）。 */
export function shortcutsAllowsDragOpen(pluginId: string): boolean {
  return pluginId.startsWith("com.window-hub.transfer-station");
}

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
  islandGap: number = SHORTCUTS_ISLAND_GAP,
): ShortcutsBounds {
  const gap = Math.max(SHORTCUTS_ISLAND_GAP_MIN, islandGap);
  const x = settingsRight + SHORTCUTS_LEFT_INSET;
  const rightMax = islandLeft - gap;
  const maxExpandWidth = Math.max(0, rightMax - x);
  return {
    x,
    width: maxExpandWidth,
    height: SHORTCUTS_HEIGHT,
    maxExpandWidth,
  };
}
