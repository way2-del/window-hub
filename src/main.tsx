import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SettingsApp from "./SettingsApp";
import TrayPopupApp from "./TrayPopupApp";
import StatusMenuPopupApp from "./StatusMenuPopupApp";
import PluginPopupHost from "./components/PluginPopupHost";
import { applyGlassCss } from "./glassPrefs";
import "./App.css";
import "./settings.css";

declare global {
  interface Window {
    __WH_IS_SETTINGS__?: boolean;
    __WH_IS_TRAY_POPUP__?: boolean;
    __WH_IS_STATUS_MENU_POPUP__?: boolean;
    __WH_IS_PLUGIN_POPUP__?: boolean;
    __WH_PLUGIN_ID__?: string;
  }
}

type WindowKind = "island" | "settings" | "tray" | "status-menu" | "plugin-popup";

function resolveWindowKind(): WindowKind {
  if (window.__WH_IS_PLUGIN_POPUP__ === true) return "plugin-popup";
  if (window.__WH_IS_STATUS_MENU_POPUP__ === true) return "status-menu";
  if (window.__WH_IS_TRAY_POPUP__ === true) return "tray";
  if (window.__WH_IS_SETTINGS__ === true) return "settings";
  try {
    const label = getCurrentWindow().label;
    if (label === "plugin-popup") return "plugin-popup";
    if (label === "status-menu-popup") return "status-menu";
    if (label === "tray-popup") return "tray";
    if (label === "settings") return "settings";
  } catch {
    /* ignore */
  }
  const q = new URLSearchParams(window.location.search).get("window");
  if (q === "plugin-popup") return "plugin-popup";
  if (q === "status-menu") return "status-menu";
  if (q === "tray") return "tray";
  if (q === "settings") return "settings";
  return "island";
}

const kind = resolveWindowKind();
document.documentElement.style.background = "transparent";
document.body.classList.add(
  kind === "settings"
    ? "is-settings"
    : kind === "tray"
      ? "is-tray-popup"
      : kind === "status-menu"
        ? "is-status-menu-popup"
        : kind === "plugin-popup"
          ? "is-plugin-popup"
          : "is-island",
);
document.body.style.background = "transparent";
document.title =
  kind === "settings"
    ? "灵动岛设置"
    : kind === "tray"
      ? "已收纳"
      : kind === "status-menu"
        ? "状态菜单"
        : kind === "plugin-popup"
          ? "插件"
          : "灵动岛";

const root = document.getElementById("root") as HTMLElement;
if (kind === "settings") {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
  root.innerHTML =
    '<div style="padding:24px;color:var(--glass-fg,#f4f4f5);font-family:Segoe UI,sans-serif;background:var(--glass-panel-bg,rgba(32,32,34,0.55));min-height:100vh">正在加载设置…</div>';
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    {kind === "settings" ? (
      <SettingsApp />
    ) : kind === "tray" ? (
      <TrayPopupApp />
    ) : kind === "status-menu" ? (
      <StatusMenuPopupApp />
    ) : kind === "plugin-popup" ? (
      <PluginPopupHost />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
