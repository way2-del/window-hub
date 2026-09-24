---
name: window-hub-island-panel
description: >-
  Window Hub 灵动岛下拉面板 — 固定内边距/圆角 token、size clamps、onEnter/onLeave、
  requestSize、plugin panel、panelWidth/panelHeight。改岛下拉 UI 时必读。
---

# 灵动岛面板（island.panel）

Slot：`island.panel`。展开内容由插件 Web 入口提供（iframe）；**无 Host 内置天气/镜子面板**。

**插件面板尺寸**：`hub.settings.panelWidth` / `panelHeight`（`settings[]`）→ 否则 `slots["island.panel"].defaultSize` → 否则 380×220。文件搜索默认 **560×400**；中转站默认高 **152**、宽 560。

**资源文件名（硬约定）：** Host 只注入同目录 **`panel.css` / `panel.js`**（与 `entry.panel` 旁路），勿只提供 `index.html`。

源码契约：

- 槽位解析：`src/plugins/islandSlots.ts`（`island.bar` / `island.drop` / `resolvePanelDefaultSize`）
- 宽高常量：`src/islandPrefs.ts` → `STAGING_PANEL_W_*` / `STAGING_PANEL_H_*` / clamp*
- 宿主形状/运动纯策略：`src/features/island/{geometry,motion,pullContent}.ts`；插件任务只查询契约，不修改这些系统实现
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
7. **禁止 iframe 再套圆角**：`.panel-plugin-frame` 始终 `border-radius: 0`；预览/卡片圆角由插件内层（如镜子 `.mirror-frame`）自绘，避免 Host 底角大圆角把上下裁成不一致。
8. **`excludeFromPullContent: true`** 的面板不进「下拉内容」；`panelOverride` / `hub.panel.openSession` 仅拖入或点岛栏时临时打开，收起必须清会话。
9. **槽位门控**：DnD → `resolveIslandDropPluginId()`（需 `island.drop` + `staging`）；岛栏 → `hub.island.setBar`（需 capability+slot `island.bar`）；全局 `barResident` 决定常驻层；`excludeFromBarResident` 不进设置列表。
10. **情景临时**（`slots["island.scenario"]`）：`claimScenario` / `releaseScenario` 暂代岛栏 + 下拉，**不改** prefs。优先级：staging overlay > scenario > resident；pull：scenario > session > prefs。有 scenario 槽的插件不进常驻/下拉竞选。Host **存在门禁**与 **打开应用托盘** 在「已安装插件详情」统一配置（全局「情景临时」仅跳转入口）。示例：正在播放。
11. **折叠岛宽自适应**：`slots["island.bar"].adaptiveWidth: true`（可选 `minWidth`/`maxWidth`）。Host 按摘要文案测量并改折叠宽（默认约 220–560）；长歌词等用此能力，**勿**在插件内改岛壳几何。源码：`resolveIslandBarAdaptive` / `measureIslandBarLabelWidth`（`islandSlots.ts`）+ `App.tsx` `liveCollapsed`。

## 生命周期

```ts
hub.island.claimScenario()
hub.island.releaseScenario()
hub.island.getBoundTray()   // → pin_key | null（插件 settings.openTrayKey）
hub.island.openBoundTray()  // 左键绑定托盘，打开对应应用
hub.island.setBar({ text, title? })
hub.island.clearBar()
hub.panel.openSession()
hub.panel.closeSession()
hub.panel.close()
hub.panel.onEnter(cb)   // 岛完全展开后
hub.panel.onLeave(cb)   // 收起一开始（摄像头等重资源必须在此 stop）
```

**摄像头 / 媒体：** 禁止在面板脚本加载时 `getUserMedia`；只在 `onEnter` 打开，`onLeave` 关闭。折叠态 Host 不发 enter。

**打开应用：** 仅 `island.scenario` 插件。绑定写在插件 `settings.openTrayKey`（插件详情自定义下拉，`uiHidden`）；面板调 `openBoundTray`。

## 尺寸

| 内容 | 宽 × 高 |
|------|---------|
| 默认 / 镜子 | `defaultSize` 或 380 × 220 |
| 天气 | `defaultSize` **380 × 248**（含岛栏；勿再压到 ≤200，三卡会被裁） |
| 中转站 | `settings.panelWidth` × `settings.panelHeight`（默认 560×152） |

## 官方插件

| 插件 | id | 槽位 |
|------|-----|------|
| 天气 | `com.window-hub.weather` | `island.bar` + `island.panel`；配置在 `settings[]`；数据 `hub.storage` |
| 镜子 | `com.window-hub.mirror` | 仅 `island.panel`；`onEnter` 开摄像头 / `onLeave` 关；iframe `allow="camera"` |
| 正在播放 | `com.window-hub.now-playing` | `island.scenario` + `island.bar.adaptiveWidth` + `island.panel`；健康时 claim 暂代岛栏/下拉 |
| 文件搜索 | `com.window-hub.file-search` | Alt+空格搜索：Everything 下拉 |

示例目录：`docs/plugins/examples/{weather,mirror,now-playing}/`（资源镜像 `src-tauri/resources/plugins/`）。首次启动缺失则 `ensure_official_plugins` 安装，不覆盖已装版本。

## Related

- 总流程：`window-hub-plugin`
- 中转站示例：`window-hub-transfer-station`
- 沉浸：`window-hub-island-immerse`
