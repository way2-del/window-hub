// Host-injected flags shared by window entrypoints.
import type { WindowFlags } from "./windowRouting";

declare global {
  interface Window extends WindowFlags {
    __WH_SETTINGS_FOCUS_PLUGIN__?: string;
    __WH_SETTINGS_FOCUS_NAV__?: string;
    __WH_WIFI_AUTH_SSID__?: string;
    __WH_DOCK_ICON_EDITOR_FOCUS__?: string;
    __WH_PLUGIN_ID__?: string;
  }
}

