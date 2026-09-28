import React from "react";
import { StartupSurface } from "./app/StartupSurface";
import { resolveWindowKind } from "./app/windowRouting";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import SettingsApp from "./SettingsApp";
import TrayPopupApp from "./TrayPopupApp";
import StatusMenuPopupApp from "./StatusMenuPopupApp";
import InputLangPopupApp from "./InputLangPopupApp";
import ChromeHoverTipApp from "./ChromeHoverTipApp";
import ControlCenterPopup from "./features/controlCenter/ControlCenterPopup";
import WifiPopupApp from "./WifiPopupApp";
import WifiAuthPopupApp from "./WifiAuthPopupApp";
import PluginPopupHost from "./components/PluginPopupHost";
import DockApp from "./DockApp";
import DockGlassApp from "./DockGlassApp";
import DockIconEditorApp from "./DockIconEditorApp";
import DockAddIconPopupApp from "./DockAddIconPopupApp";
import { applyGlassCss } from "./glassPrefs";
import "./App.css";
import "./settings.css";

const kind = resolveWindowKind(window, () => getCurrentWindow().label, window.location.search);
document.documentElement.style.background = "transparent";
const bodyClass = kind === "control-center" ? "is-control-center-popup" :
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
                  : kind === "dock-add-icon"
                    ? "is-dock-add-icon-popup"
                  : kind === "dock-icon-editor"
                    ? ["is-dock-icon-editor", "is-settings"]
                    : kind === "dock" || kind === "dock-glass" || kind === "island-bar-glass"
                      ? "is-dock"
                      : "is-island";
if (Array.isArray(bodyClass)) {
  document.body.classList.add(...bodyClass);
} else {
  document.body.classList.add(bodyClass);
}
document.body.style.background = "transparent";
document.title = kind === "control-center" ? "控制中心" :
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
                    : kind === "island-bar-glass"
                      ? "Island Bar Glass"
                      : kind === "dock-add-icon"
                        ? "添加图标"
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
  kind === "island-bar-glass" ||
  kind === "chrome-tip" ||
  kind === "input-lang" ||
  kind === "control-center" ||
  kind === "wifi" ||
  kind === "wifi-auth" ||
  kind === "dock-add-icon"
) {
  applyGlassCss({ kind: "mica-alt", dark: true, acrylicAlpha: 125 });
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <StartupSurface>
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
    ) : kind === "control-center" ? (
      <ControlCenterPopup />
    ) : kind === "wifi" ? (
      <WifiPopupApp />
    ) : kind === "wifi-auth" ? (
      <WifiAuthPopupApp />
    ) : kind === "plugin-popup" ? (
      <PluginPopupHost />
    ) : kind === "dock-add-icon" ? (
      <DockAddIconPopupApp />
    ) : kind === "dock-icon-editor" ? (
      <DockIconEditorApp />
    ) : kind === "dock" ? (
      <DockApp />
    ) : kind === "dock-glass" || kind === "island-bar-glass" ? (
      <DockGlassApp />
    ) : (
      <App />
    )}
    </StartupSurface>
  </React.StrictMode>,
);
