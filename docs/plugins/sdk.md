# Plugin SDK（当前约定）

插件包是静态资源（HTML/CSS/JS）。**禁止**未审核原生 DLL。系统能力只经 Host 注入的 `window.hub`。

开发全流程：`.cursor/skills/window-hub-plugin/SKILL.md`。

## UI：禁止浏览器原生弹窗

- **禁止** `window.alert` / `window.confirm` / `window.prompt`
- **必须**用应用内遮罩（窗口组：`.wg-confirm`）

## 三种入口怎么加载

| 入口 | Host | 旁路资源文件名 |
|------|------|----------------|
| `entry.popup` | App 壳 `?window=plugin-popup` + Tauri IPC | **固定** `popup.css` / `popup.js`（挂 `#app.wg-shell`；边距见 `window-hub-plugin-popup`） |
| `entry.panel` | 岛内 `IslandPanelHost` srcdoc | **固定** `panel.css` / `panel.js` |
| `entry.shortcuts` | 顶栏 iframe srcdoc | **stem**：`{name}.css` / `{name}.js` |

Panel / shortcuts 经 **postMessage 桥**注入 `window.hub`（可信父页转发 invoke）。Popup 用 Tauri `initialization_script`。  
这是当前正式模型，不是废弃路径。

## `window.hub` 分表面（必读）

```ts
hub.pluginId: string
hub.storage.get/set/remove/listKeys
hub.settings.getAll/get/set/subscribe

hub.windows.list/get/focus/subscribe

// Panel + Popup：
hub.staging.*          // 需 staging
hub.island.setBar/clearBar
hub.island.claimScenario/releaseScenario   // 需 island.scenario
hub.island.getBoundTray/openBoundTray       // 插件 settings.openTrayKey → 左键托盘
hub.island.onBarClick(cb)   // Shortcuts：岛栏摘要点击（无 panel 时仍触发）
hub.panel.openSession/closeSession/close

// Shortcuts iframe：
hub.shortcuts.getBounds() → { height, barHeight, width, maxExpandWidth }
hub.shortcuts.requestSize({ width })
hub.popup.open/close
hub.foreground.subscribe

// Popup only（回退 chip）：
hub.shortcuts.setBadge(badge)

// DEPRECATED — do not use:
// hub.shortcuts.setPins / clearPins
```

**已注入：** `hub.notify` / `hub.notify.onAction`、`hub.fetch`、`hub.webview.*`（三表面均有；`fetch` / `webview` 需对应 capability）。  
**未注入（勿调用）：** `hub.clipboard.*`。

```ts
hub.notify({
  title, body?, iconPng?, urgency?, ttlMs?, data?,
  defaultActionId?, // 点中部 → onAction；省略则 dismiss
  actions?: [{ id, slot: "start"|"end", label? /* 2字 */, iconPng?, background, data? }]
})
hub.notify.onAction((ev) => { /* ev.actionId / ev.data */ })
hub.fetch(url, { method?, headers?, body?, timeoutMs? }?)
hub.webview.open / close / navigate / startPick / snapshot
hub.webview.watch.start / stop / list
hub.webview.onChanged / onScanned / onClosed / onPick
hub.webview.takeLastPick()
```

通知按钮由 Host 追加在文案后并整体居中。`hub.fetch` 须声明 `network` + `permissions.network`。`hub.webview` 须声明 `webview`。

快捷区高度用 `getBounds().height` 或 `var(--wh-bar-h)`，禁止写死像素。细则：`.cursor/skills/window-hub-shortcuts/SKILL.md`。

## CapGate

`capabilities` 必须覆盖所用 API。敏感项（`windows.focus`、`staging`、`network`、`notify`、`everything.search`、`webview` 等）在安装确认弹层明示。

Companion 脚本不在 CapGate 内，见 [companion-scripts.md](./companion-scripts.md)。

## 官方示例

| 插件 | 路径 | 表面 |
|------|------|------|
| 窗口组 | `examples/window-groups/` | shortcuts + popup |
| 应用库 | `examples/app-library/` | shortcuts + popup |
| 网页监测 | `examples/page-watch/` | shortcuts + popup + notify + webview |
| 中转站 | `examples/transfer-station/` | drop + bar + panel |
| 天气 | `examples/weather/` | bar + panel（隐形 shortcuts worker） |
| 成语 | `examples/idiom/` | shortcuts 自画 chip（点击切换） |
