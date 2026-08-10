export type { PluginManifest, PluginCapability, ShortcutItem, HubWindowInfo } from "./types";
export { pluginRegistry } from "./registry";
export { assertCapability, describeCapabilities, isSensitiveCapability } from "./capGate";
export { listWindows, getWindow, focusWindow, subscribeWindows } from "./windowsApi";
export {
  SHORTCUTS_LEFT_INSET,
  SHORTCUTS_ISLAND_GAP,
  SHORTCUTS_HEIGHT,
  STATUS_MENU_BAR_HEIGHT,
  computeShortcutsBounds,
} from "./shortcutsGeometry";
export {
  ISLAND_VIEW_W,
  ISLAND_VIEW_H,
  ISLAND_BAR_H,
  ISLAND_SHELL_RADIUS,
  ISLAND_PANEL_INSET,
  ISLAND_PANEL_INSET_TOP,
  ISLAND_PANEL_RADIUS,
} from "./islandPanelGeometry";
export { islandNotifyBus } from "./islandNotify";
export { hubNotify } from "./notifyApi";
export { listPanelProviders, isBuiltinPanel } from "./panelProviders";
export { listBarResidentProviders } from "./islandSlots";
export { bootstrapPlugins, subscribeInstalledPlugins } from "./bootstrap";
