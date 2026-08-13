/** Built-in example plugins surfaced as the local “plugin market”. */

export type MarketCategoryId = "shortcuts" | "island";

export type MarketPlugin = {
  exampleId: string;
  pluginId: string;
  name: string;
  version: string;
  description: string;
  /** Short line under the name on category cards. */
  summary: string;
  categoryId: MarketCategoryId;
  tint: string;
  letter: string;
};

export type MarketCategory = {
  id: MarketCategoryId;
  name: string;
  description: string;
  blurb: string;
  tint: string;
};

export const MARKET_CATEGORIES: MarketCategory[] = [
  {
    id: "shortcuts",
    name: "快捷区",
    description: "状态菜单左侧快捷入口与弹窗",
    blurb: "从快捷区一键打开管理面板",
    tint: "#ff9f0a",
  },
  {
    id: "island",
    name: "灵动岛",
    description: "岛栏摘要、下拉面板与拖放",
    blurb: "在灵动岛上展示与交互",
    tint: "#0a84ff",
  },
];

export const MARKET_PLUGINS: MarketPlugin[] = [
  {
    exampleId: "window-groups",
    pluginId: "com.window-hub.window-groups",
    name: "窗口组",
    version: "1.2.5",
    description: "命名窗口组，从快捷区下拉弹窗切换与管理。",
    summary: "快捷区窗口组切换",
    categoryId: "shortcuts",
    tint: "#0a84ff",
    letter: "窗",
  },
  {
    exampleId: "app-library",
    pluginId: "com.window-hub.app-library",
    name: "应用库",
    version: "1.0.1",
    description: "快捷区悬浮入口打开应用库；自行添加常用应用，点击切到已打开窗口。",
    summary: "常用应用快速切换",
    categoryId: "shortcuts",
    tint: "#30d158",
    letter: "库",
  },
  {
    exampleId: "idiom",
    pluginId: "com.window-hub.idiom",
    name: "成语",
    version: "1.2.2",
    description:
      "快捷区随机成语（带声调拼音）；悬停看释义，点击切换；管理钮打开历史弹窗。",
    summary: "快捷区成语与历史",
    categoryId: "shortcuts",
    tint: "#ff9f0a",
    letter: "成",
  },
  {
    exampleId: "transfer-station",
    pluginId: "com.window-hub.transfer-station",
    name: "中转站",
    version: "1.5.1",
    description: "拖入文件后展开横向预览；弹出宽高在本插件设置中配置。",
    summary: "岛上文件中转",
    categoryId: "island",
    tint: "#64d2ff",
    letter: "转",
  },
  {
    exampleId: "weather",
    pluginId: "com.window-hub.weather",
    name: "天气",
    version: "1.0.2",
    description: "岛栏常驻天气摘要；下拉查看详细天气。开发者 ID / KEY 在本插件设置中配置。",
    summary: "岛栏天气摘要",
    categoryId: "island",
    tint: "#5ac8fa",
    letter: "天",
  },
  {
    exampleId: "now-playing",
    pluginId: "com.window-hub.now-playing",
    name: "正在播放",
    version: "1.0.3",
    description:
      "对接本机 Now Playing 服务：岛栏滚动歌词，下拉迷你播放器（封面 / 进度 / 媒体键）。",
    summary: "岛栏歌词与播放器",
    categoryId: "island",
    tint: "#bf5af2",
    letter: "播",
  },
  {
    exampleId: "mirror",
    pluginId: "com.window-hub.mirror",
    name: "镜子",
    version: "1.0.2",
    description: "灵动岛下拉摄像头预览（仅面板）。",
    summary: "岛上下拉摄像头",
    categoryId: "island",
    tint: "#8e8e93",
    letter: "镜",
  },
];

export function marketPluginsInCategory(categoryId: MarketCategoryId): MarketPlugin[] {
  return MARKET_PLUGINS.filter((p) => p.categoryId === categoryId);
}

export function findMarketPlugin(exampleIdOrPluginId: string): MarketPlugin | undefined {
  const key = exampleIdOrPluginId.trim();
  return MARKET_PLUGINS.find(
    (p) => p.exampleId === key || p.pluginId === key || p.pluginId === key.replace(/__dev$/, ""),
  );
}

export function isMarketPluginInstalled(
  pluginId: string,
  installedIds: Iterable<string>,
): boolean {
  const base = pluginId.replace(/__dev$/, "");
  for (const id of installedIds) {
    const cur = id.replace(/__dev$/, "");
    if (cur === base) return true;
  }
  return false;
}

export function searchMarketPlugins(query: string): MarketPlugin[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return MARKET_PLUGINS.filter(
    (p) =>
      p.name.toLowerCase().includes(q) ||
      p.description.toLowerCase().includes(q) ||
      p.summary.toLowerCase().includes(q) ||
      p.exampleId.includes(q) ||
      p.pluginId.toLowerCase().includes(q),
  );
}
