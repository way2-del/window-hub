import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SettingsApp from "./SettingsApp";
import TrayPopupApp from "./TrayPopupApp";
import StatusMenuPopupApp from "./StatusMenuPopupApp";
import PluginPopupHost from "./components/PluginPopupHost";
import SystemFlyoutApp from "./SystemFlyoutApp";
import DockApp from "./DockApp";
import DockItemMenuApp from "./DockItemMenuApp";
import DockPreviewApp from "./DockPreviewApp";
import SousouApp from "./SousouApp";
import { applyGlassCss } from "./glassPrefs";
import "./App.css";
import "./settings.css";
import "./systemFlyout.css";

declare global {
  interface Window {
    __WH_IS_SETTINGS__?: boolean;
    __WH_IS_TRAY_POPUP__?: boolean;
    __WH_IS_STATUS_MENU_POPUP__?: boolean;
    __WH_IS_SYSTEM_FLYOUT__?: boolean;
    __WH_SYSTEM_FLYOUT_KIND__?: string;
    __WH_IS_PLUGIN_POPUP__?: boolean;
    __WH_IS_DOCK__?: boolean;
    __WH_IS_DOCK_GLASS__?: boolean;
    __WH_IS_DOCK_ITEM_MENU__?: boolean;
    __WH_IS_DOCK_PREVIEW__?: boolean;
    __WH_IS_DOCK_TRIGGER__?: boolean;
    __WH_IS_SOUSOU__?: boolean;
    __WH_PLUGIN_ID__?: string;
  }
}

type WindowKind =
  | "island"
  | "settings"
  | "tray"
  | "status-menu"
  | "system-flyout"
  | "plugin-popup"
  | "dock"
  | "dock-glass"
  | "dock-item-menu"
  | "dock-preview"
  | "dock-trigger"
  | "sousou";

function resolveWindowKind(): WindowKind {
  if (window.__WH_IS_SOUSOU__ === true) return "sousou";
  if (window.__WH_IS_DOCK_TRIGGER__ === true) return "dock-trigger";
  if (window.__WH_IS_DOCK_PREVIEW__ === true) return "dock-preview";
  if (window.__WH_IS_DOCK_ITEM_MENU__ === true) return "dock-item-menu";
  if (window.__WH_IS_DOCK_GLASS__ === true) return "dock-glass";
  if (window.__WH_IS_DOCK__ === true) return "dock";
  if (window.__WH_IS_PLUGIN_POPUP__ === true) return "plugin-popup";
  if (window.__WH_IS_SYSTEM_FLYOUT__ === true) return "system-flyout";
  if (window.__WH_IS_STATUS_MENU_POPUP__ === true) return "status-menu";
  if (window.__WH_IS_TRAY_POPUP__ === true) return "tray";
  if (window.__WH_IS_SETTINGS__ === true) return "settings";
  try {
    const label = getCurrentWindow().label;
    if (label === "sousou") return "sousou";
    if (label === "dock-trigger") return "dock-trigger";
    if (label === "dock-preview") return "dock-preview";
    if (label === "dock-item-menu") return "dock-item-menu";
    if (label === "dock-glass") return "dock-glass";
    if (label === "dock") return "dock";
    if (label === "plugin-popup") return "plugin-popup";
    if (label === "system-flyout") return "system-flyout";
    if (label === "status-menu-popup") return "status-menu";
    if (label === "tray-popup") return "tray";
    if (label === "settings") return "settings";
  } catch {
    /* ignore */
  }
  const q = new URLSearchParams(window.location.search).get("window");
  if (q === "sousou") return "sousou";
  if (q === "dock-trigger") return "dock-trigger";
  if (q === "dock-preview") return "dock-preview";
  if (q === "dock-item-menu") return "dock-item-menu";
  if (q === "dock-glass") return "dock-glass";
  if (q === "dock") return "dock";
  if (q === "plugin-popup") return "plugin-popup";
  if (q === "system-flyout") return "system-flyout";
  if (q === "status-menu") return "status-menu";
  if (q === "tray") return "tray";
  if (q === "settings") return "settings";
  return "island";
}

const kind = resolveWindowKind();
const glassCompat =
  document.documentElement.dataset.glassCompat === "1" ||
  (window as Window & { __WH_GLASS_COMPAT__?: boolean }).__WH_GLASS_COMPAT__ === true;
const overlayOpaque =
  glassCompat &&
  (kind === "settings" ||
    kind === "tray" ||
    kind === "system-flyout" ||
    kind === "status-menu" ||
    kind === "plugin-popup" ||
    kind === "dock-item-menu" ||
    kind === "dock-preview");

// Win10 hard-safe: never force transparent over opaque HWND (white zombie).
if (kind === "sousou") {
  document.documentElement.style.background = "#f3f4f6";
  document.body.style.background = "#f3f4f6";
} else if (!overlayOpaque) {
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
} else {
  const bg =
    getComputedStyle(document.documentElement)
      .getPropertyValue("--glass-panel-bg")
      .trim() || "rgb(28, 28, 30)";
  document.documentElement.style.background = bg;
  document.body.style.background = bg;
  applyGlassCss({ kind: "mica-alt", dark: null }, undefined, true);
}

document.body.classList.add(
  kind === "sousou"
    ? "is-sousou"
    : kind === "settings"
      ? "is-settings"
      : kind === "tray"
        ? "is-tray-popup"
        : kind === "system-flyout"
          ? "is-system-flyout"
          : kind === "status-menu"
            ? "is-status-menu-popup"
            : kind === "plugin-popup"
              ? "is-plugin-popup"
              : kind === "dock-item-menu"
                ? "is-dock-item-menu"
                : kind === "dock-preview"
                  ? "is-dock-preview"
                  : kind === "dock" || kind === "dock-glass" || kind === "dock-trigger"
                    ? "is-dock"
                    : "is-island",
);
document.title =
  kind === "sousou"
    ? "搜搜"
    : kind === "settings"
      ? "灵动岛设置"
      : kind === "tray"
        ? "已收纳"
        : kind === "system-flyout"
          ? "系统面板"
          : kind === "status-menu"
            ? "状态菜单"
            : kind === "plugin-popup"
              ? "插件"
              : kind === "dock-item-menu"
                ? "Dock 菜单"
                : kind === "dock-preview"
                  ? "Dock 预览"
                  : kind === "dock-trigger" || kind === "dock-glass" || kind === "dock"
                    ? ""
                    : "灵动岛";

const root = document.getElementById("root") as HTMLElement;
if (kind === "settings") {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
  root.innerHTML =
    '<div style="padding:24px;color:var(--glass-fg,#f4f4f5);font-family:Segoe UI,sans-serif;background:var(--glass-panel-bg,rgba(32,32,34,0.55));min-height:100vh">正在加载设置…</div>';
}
if (
  kind === "dock" ||
  kind === "dock-glass" ||
  kind === "dock-trigger" ||
  kind === "dock-item-menu" ||
  kind === "dock-preview"
) {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    {kind === "sousou" ? (
      <SousouApp />
    ) : kind === "settings" ? (
      <SettingsApp />
    ) : kind === "tray" ? (
      <TrayPopupApp />
    ) : kind === "system-flyout" ? (
      <SystemFlyoutApp />
    ) : kind === "status-menu" ? (
      <StatusMenuPopupApp />
    ) : kind === "plugin-popup" ? (
      <PluginPopupHost />
    ) : kind === "dock-item-menu" ? (
      <DockItemMenuApp />
    ) : kind === "dock-preview" ? (
      <DockPreviewApp />
    ) : kind === "dock" ? (
      <DockApp />
    ) : kind === "dock-trigger" ? (
      <div
        aria-hidden
        className="dock-trigger-strip"
        style={{
          width: "100%",
          height: "100%",
          margin: 0,
          padding: 0,
          border: "none",
          borderRadius: 999,
          // Subtle edge hint — avoid opaque white slab that reads as a second taskbar.
          background:
            "linear-gradient(180deg, rgba(255,255,255,0.22) 0%, rgba(255,255,255,0.08) 100%)",
          boxShadow: "0 0 6px rgba(0,0,0,0.18)",
          pointerEvents: "none",
        }}
      />
    ) : kind === "dock-glass" ? (
      // Empty — Host SWCA paints the glass strip; icons live in `dock`.
      <div
        aria-hidden
        style={{
          width: "100%",
          height: "100%",
          margin: 0,
          padding: 0,
          border: "none",
          background: "transparent",
          pointerEvents: "none",
        }}
      />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
