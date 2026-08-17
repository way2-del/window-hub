import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SettingsApp from "./SettingsApp";
import TrayPopupApp from "./TrayPopupApp";
import StatusMenuPopupApp from "./StatusMenuPopupApp";
import InputLangPopupApp from "./InputLangPopupApp";
import ChromeHoverTipApp from "./ChromeHoverTipApp";
import WifiPopupApp from "./WifiPopupApp";
import WifiAuthPopupApp from "./WifiAuthPopupApp";
import PluginPopupHost from "./components/PluginPopupHost";
import DockApp from "./DockApp";
import DockGlassApp from "./DockGlassApp";
import DockIconEditorApp from "./DockIconEditorApp";
import { applyGlassCss } from "./glassPrefs";
import "./App.css";
import "./settings.css";

declare global {
  interface Window {
    __WH_IS_SETTINGS__?: boolean;
    __WH_SETTINGS_FOCUS_PLUGIN__?: string;
    __WH_IS_TRAY_POPUP__?: boolean;
    __WH_IS_STATUS_MENU_POPUP__?: boolean;
    __WH_IS_INPUT_LANG_POPUP__?: boolean;
    __WH_IS_CHROME_HOVER_TIP__?: boolean;
    __WH_IS_WIFI_POPUP__?: boolean;
    __WH_IS_WIFI_AUTH_POPUP__?: boolean;
    __WH_WIFI_AUTH_SSID__?: string;
    __WH_IS_PLUGIN_POPUP__?: boolean;
    __WH_IS_DOCK__?: boolean;
    __WH_IS_DOCK_GLASS__?: boolean;
    __WH_IS_DOCK_ICON_EDITOR__?: boolean;
    __WH_DOCK_ICON_EDITOR_FOCUS__?: string;
    __WH_PLUGIN_ID__?: string;
  }
}

type WindowKind =
  | "island"
  | "settings"
  | "tray"
  | "status-menu"
  | "input-lang"
  | "chrome-tip"
  | "wifi"
  | "wifi-auth"
  | "plugin-popup"
  | "dock"
  | "dock-glass"
  | "dock-icon-editor";

function resolveWindowKind(): WindowKind {
  if (window.__WH_IS_DOCK_ICON_EDITOR__ === true) return "dock-icon-editor";
  if (window.__WH_IS_DOCK_GLASS__ === true) return "dock-glass";
  if (window.__WH_IS_DOCK__ === true) return "dock";
  if (window.__WH_IS_PLUGIN_POPUP__ === true) return "plugin-popup";
  if (window.__WH_IS_WIFI_AUTH_POPUP__ === true) return "wifi-auth";
  if (window.__WH_IS_WIFI_POPUP__ === true) return "wifi";
  if (window.__WH_IS_CHROME_HOVER_TIP__ === true) return "chrome-tip";
  if (window.__WH_IS_INPUT_LANG_POPUP__ === true) return "input-lang";
  if (window.__WH_IS_STATUS_MENU_POPUP__ === true) return "status-menu";
  if (window.__WH_IS_TRAY_POPUP__ === true) return "tray";
  if (window.__WH_IS_SETTINGS__ === true) return "settings";
  try {
    const label = getCurrentWindow().label;
    if (label === "dock-icon-editor") return "dock-icon-editor";
    if (label === "dock-glass") return "dock-glass";
    if (label === "dock") return "dock";
    if (label === "plugin-popup" || label === "plugin-window") return "plugin-popup";
    if (label === "wifi-auth-popup") return "wifi-auth";
    if (label === "wifi-popup") return "wifi";
    if (label === "chrome-hover-tip") return "chrome-tip";
    if (label === "input-lang-popup") return "input-lang";
    if (label === "status-menu-popup") return "status-menu";
    if (label === "tray-popup") return "tray";
    if (label === "settings") return "settings";
  } catch {
    /* ignore */
  }
  const q = new URLSearchParams(window.location.search).get("window");
  if (q === "dock-icon-editor") return "dock-icon-editor";
  if (q === "dock-glass") return "dock-glass";
  if (q === "dock") return "dock";
  if (q === "plugin-popup" || q === "plugin-window") return "plugin-popup";
  if (q === "wifi-auth") return "wifi-auth";
  if (q === "wifi") return "wifi";
  if (q === "chrome-tip") return "chrome-tip";
  if (q === "input-lang") return "input-lang";
  if (q === "status-menu") return "status-menu";
  if (q === "tray") return "tray";
  if (q === "settings") return "settings";
  return "island";
}

const kind = resolveWindowKind();
document.documentElement.style.background = "transparent";
const bodyClass =
  kind === "settings"
    ? "is-settings"
    : kind === "tray"
      ? "is-tray-popup"
      : kind === "status-menu"
        ? "is-status-menu-popup"
        : kind === "input-lang"
          ? "is-input-lang-popup"
          : kind === "chrome-tip"
            ? "is-chrome-hover-tip"
            : kind === "wifi"
              ? "is-wifi-popup"
              : kind === "wifi-auth"
                ? "is-wifi-auth-popup"
                : kind === "plugin-popup"
                  ? "is-plugin-popup"
                  : kind === "dock-icon-editor"
                    ? ["is-dock-icon-editor", "is-settings"]
                    : kind === "dock" || kind === "dock-glass"
                      ? "is-dock"
                      : "is-island";
if (Array.isArray(bodyClass)) {
  document.body.classList.add(...bodyClass);
} else {
  document.body.classList.add(bodyClass);
}
document.body.style.background = "transparent";
document.title =
  kind === "settings"
    ? "灵动岛设置"
    : kind === "tray"
      ? "已收纳"
      : kind === "status-menu"
        ? "状态菜单"
        : kind === "input-lang"
          ? "输入法"
          : kind === "chrome-tip"
            ? "提示"
            : kind === "wifi"
              ? "WLAN"
              : kind === "wifi-auth"
                ? "加入网络"
                : kind === "plugin-popup"
                  ? "插件"
                  : kind === "dock-glass"
                    ? "Dock Glass"
                    : kind === "dock-icon-editor"
                      ? "修改图标"
                      : kind === "dock"
                        ? "Dock"
                        : "灵动岛";

const root = document.getElementById("root") as HTMLElement;
if (kind === "settings" || kind === "dock-icon-editor") {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
  root.innerHTML =
    kind === "dock-icon-editor"
      ? '<div style="padding:24px;color:#f4f4f5;font-family:Segoe UI,sans-serif;background:#1c1c1e;min-height:100vh">正在加载修改图标…</div>'
      : '<div style="padding:24px;color:#f4f4f5;font-family:Segoe UI,sans-serif;background:#1c1c1e;min-height:100vh">正在加载设置…</div>';
}
if (
  kind === "dock" ||
  kind === "dock-glass" ||
  kind === "chrome-tip" ||
  kind === "input-lang" ||
  kind === "wifi" ||
  kind === "wifi-auth"
) {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    {kind === "settings" ? (
      <SettingsApp />
    ) : kind === "tray" ? (
      <TrayPopupApp />
    ) : kind === "status-menu" ? (
      <StatusMenuPopupApp />
    ) : kind === "input-lang" ? (
      <InputLangPopupApp />
    ) : kind === "chrome-tip" ? (
      <ChromeHoverTipApp />
    ) : kind === "wifi" ? (
      <WifiPopupApp />
    ) : kind === "wifi-auth" ? (
      <WifiAuthPopupApp />
    ) : kind === "plugin-popup" ? (
      <PluginPopupHost />
    ) : kind === "dock-icon-editor" ? (
      <DockIconEditorApp />
    ) : kind === "dock" ? (
      <DockApp />
    ) : kind === "dock-glass" ? (
      <DockGlassApp />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
