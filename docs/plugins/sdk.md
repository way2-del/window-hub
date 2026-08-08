# Plugin SDK（当前约定）

插件包是静态资源（HTML/CSS/JS）。**禁止**未审核原生 DLL。系统能力只经 Host 注入的 `window.hub`。

开发全流程：`.cursor/skills/window-hub-plugin/SKILL.md`。

## UI：禁止浏览器原生弹窗

- **禁止** `window.alert` / `window.confirm` / `window.prompt`
- **必须**用应用内遮罩（窗口组：`.wg-confirm`）

## 三种入口怎么加载

| 入口 | Host | 旁路资源文件名 |
|------|------|----------------|
| `entry.popup` | App 壳 `?window=plugin-popup` + Tauri IPC | **固定** `popup.css` / `popup.js` |
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

**未注入（勿调用）：** `hub.notify`、`hub.clipboard.*`、`hub.fetch`。

快捷区高度用 `getBounds().height` 或 `var(--wh-bar-h)`，禁止写死像素。细则：`.cursor/skills/window-hub-shortcuts/SKILL.md`。

## CapGate

`capabilities` 必须覆盖所用 API。敏感项（`windows.focus`、`staging`、未来的 clipboard/network）安装 UI 明示。

Companion 脚本不在 CapGate 内，见 [companion-scripts.md](./companion-scripts.md)。

## 官方示例

| 插件 | 路径 | 表面 |
|------|------|------|
| 窗口组 | `examples/window-groups/` | shortcuts + popup |
| 中转站 | `examples/transfer-station/` | drop + bar + panel |
