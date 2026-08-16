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
  | "media.keys"
  | "clipboard.read"
  | "clipboard.write"
  | "network"
  | "staging"
  | "everything.search";

export type ShortcutsAction = "expand" | "popup.open" | "panel.open" | "command";

/**
 * 快捷区 2×2 管理/设置钮（slots.shortcuts.manage）
 * - custom：插件自画（自定义弹窗）；Host 不重复画
 * - none：不显示
 * - settings：Host 画钮，点击跳转设置页对该插件
 */
export type ShortcutsManageMode = "custom" | "none" | "settings";

export type ShortcutsSlotConfig = {
  icon: string;
  label?: string;
  order?: number;
  action?: ShortcutsAction;
  /** 管理/设置钮：custom | none | settings（默认 none） */
  manage?: ShortcutsManageMode;
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
  /** Host custom UI (e.g. tray picker); skip generic PluginSettingsForm row. */
  uiHidden?: boolean;
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
      /**
       * Host 按摘要文案自适应折叠岛宽（歌词等长文本）。
       * 可选 minWidth / maxWidth（逻辑像素，默认 220–560）。
       */
      adaptiveWidth?: boolean;
      minWidth?: number;
      maxWidth?: number;
    };
    "island.drop"?: { order?: number };
    /**
     * 情景临时：健康时可暂代岛栏 + 下拉，不改常驻/下拉 prefs。
     * 有此槽位的插件不出现在「岛栏常驻 / 下拉内容」竞选列表。
     */
    "island.scenario"?: { order?: number };
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
};
