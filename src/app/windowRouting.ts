// Pure routing policy: injected flags > native label > query > island.
// Keep native label lookup lazy: injected flags must work before Tauri is ready.
export type WindowFlags = Partial<Record<
  | "__WH_IS_DOCK_ICON_EDITOR__"
  | "__WH_IS_DOCK_GLASS__"
  | "__WH_IS_ISLAND_BAR_GLASS__"
  | "__WH_IS_DOCK__"
  | "__WH_IS_PLUGIN_POPUP__"
  | "__WH_IS_WIFI_AUTH_POPUP__"
  | "__WH_IS_CONTROL_CENTER__"
  | "__WH_IS_WIFI_POPUP__"
  | "__WH_IS_CHROME_HOVER_TIP__"
  | "__WH_IS_INPUT_LANG_POPUP__"
  | "__WH_IS_STATUS_MENU_POPUP__"
  | "__WH_IS_TRAY_POPUP__"
  | "__WH_IS_SETTINGS__", boolean>>;

export type WindowKind =
  | "island"
  | "settings"
  | "tray"
  | "status-menu"
  | "input-lang"
  | "chrome-tip"
  | "control-center"
  | "wifi"
  | "wifi-auth"
  | "plugin-popup"
  | "dock"
  | "dock-glass"
  | "island-bar-glass"
  | "dock-icon-editor";

export function resolveWindowKind(
  flags: WindowFlags,
  getLabel: () => string,
  search: string,
): WindowKind {
  if (flags.__WH_IS_DOCK_ICON_EDITOR__ === true) return "dock-icon-editor";
  if (flags.__WH_IS_DOCK_GLASS__ === true) return "dock-glass";
  if (flags.__WH_IS_ISLAND_BAR_GLASS__ === true) return "island-bar-glass";
  if (flags.__WH_IS_DOCK__ === true) return "dock";
  if (flags.__WH_IS_PLUGIN_POPUP__ === true) return "plugin-popup";
  if (flags.__WH_IS_WIFI_AUTH_POPUP__ === true) return "wifi-auth";
  if (flags.__WH_IS_CONTROL_CENTER__ === true) return "control-center";
  if (flags.__WH_IS_WIFI_POPUP__ === true) return "wifi";
  if (flags.__WH_IS_CHROME_HOVER_TIP__ === true) return "chrome-tip";
  if (flags.__WH_IS_INPUT_LANG_POPUP__ === true) return "input-lang";
  if (flags.__WH_IS_STATUS_MENU_POPUP__ === true) return "status-menu";
  if (flags.__WH_IS_TRAY_POPUP__ === true) return "tray";
  if (flags.__WH_IS_SETTINGS__ === true) return "settings";
  try {
    const label = getLabel();
    if (label === "dock-icon-editor") return "dock-icon-editor";
    if (label === "dock-glass") return "dock-glass";
    if (label === "island-bar-glass") return "island-bar-glass";
    if (label === "dock") return "dock";
    if (label === "plugin-popup" || label === "plugin-window") return "plugin-popup";
    if (label === "wifi-auth-popup") return "wifi-auth";
    if (label === "control-center-popup") return "control-center";
    if (label === "wifi-popup") return "wifi";
    if (label === "chrome-hover-tip") return "chrome-tip";
    if (label === "input-lang-popup") return "input-lang";
    if (label === "status-menu-popup") return "status-menu";
    if (label === "tray-popup") return "tray";
    if (label === "settings") return "settings";
  } catch {
    /* ignore */
  }
  const q = new URLSearchParams(search).get("window");
  if (q === "dock-icon-editor") return "dock-icon-editor";
  if (q === "dock-glass") return "dock-glass";
  if (q === "island-bar-glass") return "island-bar-glass";
  if (q === "dock") return "dock";
  if (q === "plugin-popup" || q === "plugin-window") return "plugin-popup";
  if (q === "wifi-auth") return "wifi-auth";
  if (q === "control-center") return "control-center";
  if (q === "wifi") return "wifi";
  if (q === "chrome-tip") return "chrome-tip";
  if (q === "input-lang") return "input-lang";
  if (q === "status-menu") return "status-menu";
  if (q === "tray") return "tray";
  if (q === "settings") return "settings";
  return "island";
}

