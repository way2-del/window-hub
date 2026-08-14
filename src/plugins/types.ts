/** Plugin manifest & runtime types (aligned with docs/plugins/plugin.schema.json). */

export type PluginCapability =
  | "storage"
  | "shortcuts"
  | "notify"
  | "popup"
  | "island.panel"
  | "island.bar"
  | "island.drop"
  | "windows.read"
  | "windows.focus"
  | "clipboard.read"
  | "clipboard.write"
  | "network"
  | "staging";

export type ShortcutsAction = "expand" | "popup.open" | "panel.open" | "command";

export type ShortcutsSlotConfig = {
  icon: string;
  label?: string;
  order?: number;
  action?: ShortcutsAction;
  /** Host 回退 chip：只显示图标，不显示 label */
  iconOnly?: boolean;
};

export type PluginSettingType =
  | "boolean"
  | "string"
  | "number"
  | "select"
  | "radio"
  | "multiSelect";

export type PluginSettingOption = {
  value: string | number | boolean;
  label: string;
};

export type PluginSettingField = {
  key: string;
  type: PluginSettingType;
  label: string;
  description?: string;
  default?: unknown;
  options?: PluginSettingOption[];
  min?: number;
  max?: number;
  step?: number;
  maxLength?: number;
};

export type PluginManifest = {
  id: string;
  name: string;
  version: string;
  official?: boolean;
  homepage?: string;
  minHostVersion?: string;
  logo?: string;
  description?: string;
  entry?: {
    panel?: string;
    popup?: string;
    /** Short strip HTML for status-menu shortcuts (Host iframe shell). */
    shortcuts?: string;
    development?: { panel?: string; popup?: string; shortcuts?: string };
  };
  slots?: {
    shortcuts?: ShortcutsSlotConfig;
    "island.notify"?: { priority?: string; maxPerMinute?: number };
    "island.bar"?: {
      order?: number;
      /** 不出现在「岛栏常驻」列表（仍可临时 setBar，如中转站） */
      excludeFromBarResident?: boolean;
    };
    "island.drop"?: { order?: number };
    "island.panel"?: {
      defaultSize?: { w: number; h: number };
      minSize?: { w: number; h: number };
      maxSize?: { w: number; h: number };
      /** Omit from Settings pull-content; open via drop/bar/session only */
      excludeFromPullContent?: boolean;
    };
  };
  /** Declarative settings rendered in Host Settings; values in __settings */
  settings?: PluginSettingField[];
  capabilities?: PluginCapability[];
  permissions?: { network?: string[] };
};

export type ShortcutItem = {
  id: string;
  title: string;
  subtitle?: string;
  /** Window id for focus, if applicable */
  windowId?: string;
  stale?: boolean;
};

export type ShortcutsPluginRuntime = {
  pluginId: string;
  manifest: PluginManifest;
  badge: string | number | null;
  items: ShortcutItem[];
  /** Host-owned: only one expanded at a time */
  expanded: boolean;
};

export type HubWindowInfo = {
  id: string;
  hwnd: number;
  title: string;
  class_name: string;
  pid: number;
  exe?: string | null;
  exe_name?: string | null;
};

export type IslandBarState = {
  pluginId: string;
  text: string;
  title?: string;
  /** data-URL image (desktop lyric PNG mirror) */
  image?: string;
  /** DWM live mirror slot (Host paints DesktopLyrics into bar) */
  mirror?: boolean;
  /** 歌词镜像垂直微调（逻辑像素，正数下移） */
  mirrorOffsetY?: number;
};
