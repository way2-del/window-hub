# Host API 与 CapGate

基座以 Tauri command + 注入的 `window.hub` 暴露能力。调用方身份 = 命令参数 `pluginId`（由 Host 注入，不可被页面篡改）；未声明 capability → 拒绝。

分表面注入差异见 [sdk.md](./sdk.md)。完整开发流程：`.cursor/skills/window-hub-plugin/SKILL.md`。

## CapGate

| Capability | 含义 | 状态 |
|------------|------|------|
| `storage` | `hub.storage.*` + `hub.settings.*` | ✅ |
| `shortcuts` | 快捷区槽位；条内几何 / badge | ✅（`setPins` 已废弃） |
| `popup` | 打开托管弹窗 | ✅ |
| `island.panel` | 岛下拉 + `hub.panel.openSession` | ✅ |
| `island.drop` | 参与岛 DnD 路由（须同时有 `staging`） | ✅ |
| `staging` | `hub.staging.*` 按插件隔离暂存 | ✅ |
| `windows.read` | list / get / subscribe | ✅ |
| `windows.focus` | focus（敏感，安装须明示） | ✅ |
| `notify` | `hub.notify`（需 slot `island.notify`） | ✅ Popup / Panel / Shortcuts |
| `clipboard.read` / `clipboard.write` | 剪贴板 | ❌ schema 占位，无 API |
| `network` | `hub.fetch` + `permissions.network` 白名单 | ✅ |

**槽位门控（非 capability）：** `hub.island.setBar` / `clearBar` 需 `slots["island.bar"]`。

## 共享 WindowsService

Rust 单例轮询（默认 250ms），事件 `hub-windows-changed`。多插件 `subscribe` 共享快照。

## `hub.storage.*` / 保留键

表 **`plugin_kv`**。规范：[local-storage.md](./local-storage.md)。

| 保留键 | API | 说明 |
|--------|-----|------|
| `__staging_items` | `hub.staging.*` | 按 pluginId 隔离 |
| `__settings` | `hub.settings.*` | `settings[]`；需 `storage` |
| `__shortcuts_pins` | ~~setPins~~ | **废弃**，勿写 |

## `hub.settings.*`

`plugin.json` → `settings[]`；类型 `boolean` / `string` / `number` / `select` / `radio` / `multiSelect`。  
示例：中转站 `panelWidth` / `panelHeight`。Host 面板尺寸：settings → `defaultSize` → 380×220。

## `hub.staging.*`

需 `staging`。事件 `staging-changed` → `{ pluginId, files, texts, images, total }`。  
方法：`list` / `summary` / `addText` / `addPaths` / `addImageBytes` / `remove` / `clear` / `copy` / `copyAllPaths` / `thumb` / `reveal` / `startDrag` / `subscribe`。

## `hub.island.*` / `hub.panel`

| 方法 | 说明 |
|------|------|
| `island.setBar({ text, title? })` | 需 `island.bar` |
| `island.clearBar()` | 需 `island.bar` |
| `island.onBarClick(cb)` | Shortcuts iframe：岛栏摘要被点击时回调（无 panel 也可） |
| `panel.openSession()` | 需 `island.panel`；临时展开该插件面板 |
| `panel.closeSession()` | 清会话并收起 |
| `panel.close()` | 面板内关闭 |

DnD → `island.drop` 赢家。`excludeFromPullContent: true` → 不进下拉内容列表。

## `hub.shortcuts.*`（快捷区 iframe）

| 方法 | 说明 |
|------|------|
| `getBounds()` | `{ height, barHeight, width, maxExpandWidth }`；高度恒为 **28** |
| `requestSize({ width })` | 通知 Host 条宽 |
| `setBadge` | 仅无网页入口的回退 chip |
| ~~`setPins` / `clearPins`~~ | **废弃** |

## `hub.popup.*`

`open_plugin_popup` / `close_plugin_popup`；注入后读 `popup.css` / `popup.js`。快捷区可开弹窗；岛面板 **故意** 不注入 `popup.open`。

## `hub.notify`

```ts
hub.notify({
  title: string;
  body?: string;
  iconPng?: string;
  urgency?: "passive" | "active" | "critical";
  ttlMs?: number;
  data?: unknown;
  actions?: {
    id: string;
    slot: "start" | "end";       // 仅开头/结尾，每槽最多 1
    label?: string;              // 恰好 2 字；与 iconPng 互斥
    iconPng?: string;
    background: string;          // 安全 CSS 色
    data?: unknown;
  }[];
}): Promise<{ id: string }>

hub.notify.onAction((ev: {
  notifyId: string;
  actionId: string;
  data?: unknown;
}) => void): () => void
```

需 `notify` + `slots["island.notify"]`。按钮由 Host 固定布局（垂直居中、大圆角、字号=岛栏 12px）。  
**点横幅中部**（所有插件统一）：dismiss 后若声明 `island.panel` 则下拉打开该插件面板；**不是**托盘应用跳转。左右按钮走 `onAction`。细则：`.cursor/skills/window-hub-island-notify/SKILL.md`。

## `hub.fetch`

```ts
hub.fetch(url: string, opts?: {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  timeoutMs?: number;
}): Promise<{ status: number; ok: boolean; headers: Record<string, string>; body: string }>
```

需 `network`；URL 必须匹配 `permissions.network`（主机/模式白名单）。空白名单 → 拒绝。

## Companion

Hub 未提供的能力 → 独立进程脚本 + 本地 HTTP，设置页登记自启。见 [companion-scripts.md](./companion-scripts.md)。  
**禁止**未审核原生二进制进主进程。
