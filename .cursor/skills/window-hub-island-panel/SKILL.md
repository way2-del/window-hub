---
name: window-hub-island-panel
description: >-
  Window Hub 灵动岛下拉面板 — 固定内边距/圆角 token、size clamps、onEnter/onLeave、
  requestSize、weather/mirror/plugin panel、panelWidth/panelHeight。改岛下拉 UI 时必读。
---

# 灵动岛面板（island.panel）

Slot：`island.panel`。展开内容由 Host provider 或插件 Web 入口提供；**默认尺寸由 Host 钳制**（天气/镜子 ~380×220）。

**插件面板尺寸**：`hub.settings.panelWidth` / `panelHeight`（`settings[]`）→ 否则 `slots["island.panel"].defaultSize` → 否则 380×220。中转站默认高 **152**、宽 560。**禁止按插件 id 硬编码壳型**。

**资源文件名（硬约定）：** Host 只注入同目录 **`panel.css` / `panel.js`**（与 `entry.panel` 旁路），勿只提供 `index.html`。

源码契约：

- 槽位解析：`src/plugins/islandSlots.ts`（`island.bar` / `island.drop` / `resolvePanelDefaultSize`）
- 宽高常量：`src/islandPrefs.ts` → `STAGING_PANEL_W_*` / `STAGING_PANEL_H_*` / clamp*
- 样式：`src/App.css` → `.island-panel.is-plugin-sized`

## 固定几何（禁止随意改数）

| Token | 值 | CSS 变量 | 含义 |
|-------|-----|----------|------|
| `ISLAND_SHELL_RADIUS` | **32** | `--island-shell-radius` | 展开态岛壳**底角** |
| `ISLAND_PANEL_RADIUS` | **32** | `--island-panel-radius` | 面板内预览/卡片圆角 |
| `ISLAND_PANEL_INSET` | **16** | `--island-panel-inset` | **左 = 右 = 底** 内边距 |
| `ISLAND_PANEL_INSET_TOP` | **10** | `--island-panel-inset-top` | 顶边距 |
| `ISLAND_VIEW_W/H` | 380×220 | — | 默认 Host 硬顶（非中转站） |
| `panelWidth` / `panelHeight` | 440–720 / 120–184（默认 560×152） | — | 插件面板设置 |
| `ISLAND_BAR_H` | 28 | `--island-bar-h` | 顶栏高度 |

### 硬性规则

1. **底边距必须等于左右边距**（同为 `ISLAND_PANEL_INSET`）。
2. **摄像头 / 媒体预览**圆角 = `ISLAND_PANEL_RADIUS`（32）。
3. **顶角**：岛壳顶角始终直角贴屏。
4. 插件面板 iframe 遵守同一 inset / radius。
5. **对比度**：面板 iframe 禁止跟随设置浅色主题；深色底 + 浅色字（见 `window-hub-island-immerse`）。
6. **尺寸只在插件 `settings[]` / `defaultSize`**：如 `panelWidth`、`panelHeight`，禁止放全局设置或 Host 壳型枚举。
7. **禁止 iframe 四角大圆角**：`.panel-plugin-frame` 不得四角大圆角裁切标题；`is-plugin-sized` 时 iframe `border-radius: 0`。
8. **`excludeFromPullContent: true`** 的面板不进「下拉内容」；`panelOverride` / `hub.panel.openSession` 仅拖入或点岛栏时临时打开，收起必须清会话。
9. **槽位门控**：DnD → `resolveIslandDropPluginId()`（需 `island.drop` + `staging`）；岛栏 → `hub.island.setBar` 或 Host 对 `staging-changed` 的通用同步（需 `island.bar`）；禁用对应插件后宿主忽略。

## 生命周期

```ts
hub.island.setBar({ text, title? })
hub.island.clearBar()
hub.panel.openSession()
hub.panel.closeSession()
hub.panel.close()
```

## 尺寸

| 内容 | 宽 × 高 |
|------|---------|
| 天气/镜子 | 380 × 220 |
| 中转站 | `settings.panelWidth` × `settings.panelHeight`（默认 560×152） |

## 镜子（mirror）

- 展开动画结束后再 `getUserMedia`；禁止下拉过程中提前开摄像头。

## Related

- 总流程：`window-hub-plugin`
- 中转站示例：`window-hub-transfer-station`
- 沉浸：`window-hub-island-immerse`
