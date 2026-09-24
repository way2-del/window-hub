---
name: window-hub-shortcuts
description: >-
  Window Hub 快捷区 — Host iframe strips per plugin (entry.shortcuts),
  slots.shortcuts.manage (custom|none|settings), shortcutsHubBridge,
  getBounds bar height, exclusive prefs. setPins is deprecated. Use when
  implementing ShortcutsHost or shortcuts slot plugins.
---

# 快捷区（状态菜单 · shortcuts）

隶属于 **状态菜单**。Slot ID：`shortcuts`。

## 模型

Host 在顶栏快捷区几何内 **并排挂矮 iframe**（每插件一块）。插件用 HTML/CSS/JS 自画整条（入口 + 固定项等）。  
**不要**用 `hub.shortcuts.setPins`（已废弃）。

模式对齐岛 panel：`srcdoc` + [`shortcutsHubBridge`](../../../src/plugins/shortcutsHubBridge.ts)。

## 管理/设置钮 `slots.shortcuts.manage`（必读）

快捷区左侧常见的 **2×2 宫格钮** 由 manifest **显式声明**，禁止 Host 对所有插件一律补画（会与自定义弹窗重复）。

| 值 | 中文语义 | 谁画钮 | 点击行为 |
|----|----------|--------|----------|
| `custom` | **显示**（自定义） | **插件**在 `shortcuts.js` 自画 | 插件自定（通常 `hub.popup.open`） |
| `none` | **不显示** | 无人 | — |
| `settings` | **显示并跳转设置** | **Host** 在条前画 2×2 | `open_settings_window` 聚焦该插件 |

- **缺省 / 省略** = `none`（不画）。
- `custom` 时 Host **禁止**再画一颗 2×2。
- `settings` 时插件条内 **禁止**再画同款管理钮。
- 仅有声明式 `settings[]`、无自定义弹窗 → 用 `settings`（例：成语）。
- 有自画管理弹窗（窗口组、应用库）→ 用 `custom`，条内保留 manage chip。
- 隐形 worker（天气 shortcuts 只轮询）→ `none`。

```json
"slots": {
  "shortcuts": {
    "icon": "windows",
    "label": "窗口组",
    "order": 20,
    "action": "popup.open",
    "manage": "custom"
  }
}
```

```json
"slots": {
  "shortcuts": {
    "icon": "icon.svg",
    "label": "成语",
    "order": 8,
    "action": "command",
    "manage": "settings"
  }
}
```

实现：[`ShortcutsHost.tsx`](../../../src/components/ShortcutsHost.tsx) 仅当 `manage === "settings"` 渲染 Host chip。

## 状态栏高度（必须可读）

状态菜单顶栏高度与快捷区 iframe 高度 **同一常量**：

| Token | 值 | 来源 |
|-------|-----|------|
| `SHORTCUTS_HEIGHT` / `STATUS_MENU_BAR_HEIGHT` | **28** | [`shortcutsGeometry.ts`](../../../src/plugins/shortcutsGeometry.ts) |
| CSS `--island-bar-h` | 28px | Host `.shell` |
| 注入 `--wh-bar-h` | 同上 | Host → iframe `:root` |

插件 **禁止**写死 `28px` 做布局决策。应：

```ts
const { height, barHeight } = await hub.shortcuts.getBounds();
document.documentElement.style.setProperty("--wh-bar-h", `${height}px`);
```

或直接用 Host 已注入的 `var(--wh-bar-h)`。

### 垂直对齐（常驻入口统一居中）

常驻 chip 的容器、按钮和图标以 `--wh-bar-h` 垂直居中；文字继承 Host 的 `--wh-chrome-font-*`，显式统一行高，SVG 用 `display:block; flex-shrink:0`，避免行内图标基线空隙。

验收同时比较文字与图标的视觉中心、相邻状态菜单标题的基线。中英文混排在几何居中后仍可能有字形偏差，允许在截图验证后对文字做局部 1px 光学校正；不要移动整个条或用固定 top 猜状态栏位置。高度不得超过 `getBounds().height`。

### 动态内容宽度（防延迟裁切）

- 用 `id="bar"` 标记实际内容根节点，设置 `width:max-content`；chip 禁止 flex 收缩。Host 自动量测和插件主动上报必须量测同一内容。
- 宽度取 `Math.ceil(Math.max(bar.scrollWidth, bar.getBoundingClientRect().width))`，包含图标、间隔、padding 和实际字体；不要按城市名字数估算，不要用固定上限截掉日期/星期/AM-PM。
- 初次渲染、设置切换、定时刷新都走同一测量逻辑；用 `ResizeObserver` 处理 Host 字体注入和字体加载引起的宽度变化。仅宽度变化时调用 `requestSize`，避免重复消息。
- 不要量测 `width:100%` 的 iframe 视口来当作内容宽度，否则裁切后无法恢复或缩短后无法收回。Host 可用空间不足的裁切与插件错误上报宽度需分别排查。
- 回归至少覆盖：最长组合 → 等待两次实际刷新 → 短组合 → 长组合、12/24 小时制、字体变化及不同缩放。确认右侧最后字符始终可见、宽度能扩大也能缩回。内置资源与 `docs/plugins/examples` 同步。

## Hover 开 popup

悬停打开管理/设置弹窗时 **必须 dwell**，禁止一划过就 `popup.open`。

| Token | 推荐值 | 说明 |
|-------|--------|------|
| `HOVER_OPEN_MS` | **450** | `pointerenter` → `setTimeout` → `hub.popup.open`；`pointerleave` 取消 |

点击仍可立即开/关（toggle）。官方窗口组 / 应用库 / 成语与 Host 回退 chip 均用此值。

## 允许

- 插件网页横向 chip / 分隔线 / 绿点 / 悬停开 popup（见上 dwell）
- 按 `manage` 三态画或不画 2×2（见上表）
- `hub.shortcuts.getBounds()` 读状态栏/条几何
- `hub.shortcuts.requestSize({ width })` 通知 Host 条宽
- 独占：`prefs_shortcuts.exclusivePluginId`
- 无 `entry.shortcuts` 时回退 Host 入口 chip

## 禁止

- Host 对 `manage=custom` 的插件再画一颗 2×2
- `manage=settings` 时插件条内再画同款管理钮
- 另开叠层 WebviewWindow 画快捷区
- 高度 > `SHORTCUTS_HEIGHT`（28px）
- 自建置顶窗（用 `hub.popup.open`）
- 继续依赖 `setPins` 让 Host 画 pin
- 在插件里魔法数猜状态栏高度（必须用 `getBounds` / `--wh-bar-h`）

## Manifest

```json
{
  "entry": {
    "shortcuts": "shortcuts.html",
    "popup": "popup.html"
  },
  "slots": {
    "shortcuts": {
      "icon": "windows",
      "label": "窗口组",
      "order": 20,
      "action": "popup.open",
      "manage": "custom"
    }
  },
  "capabilities": ["shortcuts", "storage", "popup", "windows.read", "windows.focus"]
}
```

## Bridge `window.hub`（iframe 内）

- `storage.*` / `windows.list|get|focus|subscribe`
- `popup.open` / `popup.close`
- `shortcuts.getBounds()` → `{ height, barHeight, width, maxExpandWidth }`
- `shortcuts.requestSize({ width })`
- `shortcuts.showTip({ lines, x, y })` / `hideTip()` — **系统统一 hover tip**（独立 overlay，与插件弹窗同一套 Mica + `--glass-panel-bg`）；快捷区 iframe 内原生 `title` 会自动改走 Host tip（`data-host-tip="off"` 可退出）
- Host 顶栏/托盘/Dock 用 [`chromeHoverTip.ts`](../../../src/chromeHoverTip.ts) `hostTipPointerProps` / `showChromeHoverTip`，禁止再用系统 `title` 气泡
- `foreground.subscribe`（绿点）
- 父页在 popup 关闭时发 `refresh`

## 实现

1. [`ShortcutsHost.tsx`](../../../src/components/ShortcutsHost.tsx) 并排 `ShortcutsPluginStrip`；`manage=settings` 时前置 Host 设置钮
2. 几何：[`shortcutsGeometry.ts`](../../../src/plugins/shortcutsGeometry.ts)
3. 官方示例：
   - [`window-groups`](../../../docs/plugins/examples/window-groups/) — `manage=custom`
   - [`app-library`](../../../docs/plugins/examples/app-library/) — `manage=custom`
   - [`idiom`](../../../docs/plugins/examples/idiom/) — `manage=custom`（历史弹窗 + pinyin-pro 带调）
