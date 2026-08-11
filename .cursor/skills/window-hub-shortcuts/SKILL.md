---
name: window-hub-shortcuts
description: >-
  Window Hub 快捷区 — Host iframe strips per plugin (entry.shortcuts),
  shortcutsHubBridge, getBounds bar height, exclusive prefs. setPins is
  deprecated. Use when implementing ShortcutsHost or shortcuts slot plugins.
---

# 快捷区（状态菜单 · shortcuts）

隶属于 **状态菜单**。Slot ID：`shortcuts`。

## 模型

Host 在顶栏快捷区几何内 **并排挂矮 iframe**（每插件一块）。插件用 HTML/CSS/JS 自画整条（入口 + 固定项等）。  
**不要**用 `hub.shortcuts.setPins`（已废弃）。

模式对齐岛 panel：`srcdoc` + [`shortcutsHubBridge`](../../../src/plugins/shortcutsHubBridge.ts)。

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
// height === barHeight === 状态栏高度（逻辑 px）
document.documentElement.style.setProperty("--wh-bar-h", `${height}px`);
```

或直接用 Host 已注入的 `var(--wh-bar-h)`。

### 垂直对齐（由插件自决）

拿到 `height` / `--wh-bar-h` 后，插件选择内容在条内的垂直对齐：

| 对齐 | CSS 做法（示例） |
|------|------------------|
| **居中**（推荐常驻 chip） | `html, body, .bar { display:flex; align-items:center; height:var(--wh-bar-h); }` |
| **顶对齐** | `align-items: flex-start` 或 `padding-top` |
| **底对齐** | `align-items: flex-end` 或 `padding-bottom` |

官方窗口组：**全部元素垂直居中**（见 `window-hub-window-groups`）。

高度硬顶：内容区不得超过 `getBounds().height`（= `SHORTCUTS_HEIGHT`）。

## 允许

- 插件网页横向 chip / 分隔线 / 绿点；点击开 popup（不悬停即开）
- `hub.shortcuts.getBounds()` 读状态栏/条几何
- `hub.shortcuts.requestSize({ width })` 通知 Host 条宽
- 可见插件：`prefs_shortcuts.visiblePluginIds`（空 = 全部；可多选）
- 无 `entry.shortcuts` 时回退 Host 入口 chip
- 隐形 worker（`action: "command"`，如天气/歌词）条宽可为 1px，勿被 Host 抬到 28

## 禁止

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
      "action": "popup.open"
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
- `foreground.subscribe`（绿点）
- 父页在 popup 关闭时发 `refresh`

## 实现

1. [`ShortcutsHost.tsx`](../../../src/components/ShortcutsHost.tsx) 并排 `ShortcutsPluginStrip`
2. 几何：[`shortcutsGeometry.ts`](../../../src/plugins/shortcutsGeometry.ts)
3. 官方示例：[`window-groups/shortcuts.*`](../../../docs/plugins/examples/window-groups/)
