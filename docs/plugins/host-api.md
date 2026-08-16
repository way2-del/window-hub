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
| `media.keys` | 系统媒体键 play_pause/next/previous/stop（敏感） | ✅ |
| `notify` | `hub.notify`（需 slot `island.notify`） | ✅ Popup / Panel / Shortcuts |
| `clipboard.read` / `clipboard.write` | 剪贴板 | ❌ schema 占位，无 API |
| `network` | `hub.fetch` + `permissions.network` 白名单 | ✅ |
| `everything.search` | Everything 本机文件搜索 / 打开 / 定位（敏感；需本机 Everything 运行） | ✅ |
| `system.monitor` | 本机 CPU / 内存 / 磁盘 / 温度快照（敏感） | ✅ |

**槽位门控（非 capability）：** `hub.island.setBar` / `clearBar` 需 `slots["island.bar"]`。  
设置「岛栏常驻」选中的插件可写折叠态摘要；`excludeFromBarResident` 插件（如中转站）可走临时覆盖层。  
`slots["island.scenario"]` → **情景临时**：`claimScenario` / `releaseScenario` 暂代岛栏 + 下拉（**不改** SQLite 常驻/下拉 prefs）；有此槽位的插件不出现在常驻/下拉竞选列表。  
显示优先级：**中转站 overlay > 情景临时 > 岛栏常驻**；下拉：**情景 pull > panelOverride > 设置下拉内容**。折叠岛不清情景。  
**存在门禁（Host）** / **打开应用（插件 settings）**：在 **已安装插件详情**（设置 → 插件市场 → 已安装 → 情景插件）统一配置；全局「情景临时」列表仅作跳转入口。`trayKeys` / `windowKeys` 写在岛 prefs `scenarioGates`；`openTrayKey` 写在插件 `settings`（`hub.island.openBoundTray`）。  
`slots["island.bar"].adaptiveWidth: true` → Host 按摘要文案自适应折叠岛宽（`minWidth`/`maxWidth`，默认 220–560）；长歌词等勿再靠 ellipsis 硬裁。

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
| `island.setBar({ text, title? })` | 需 `island.bar`；常驻主人 / 情景主人 / exclude overlay 可写对应层；若 `adaptiveWidth`，Host 按文案调折叠岛宽 |
| `island.clearBar()` | 需 `island.bar`；清本插件所在层 |
| `island.claimScenario()` | 需 `island.scenario` + bar + panel；接管岛栏与下拉（后 claim 顶替前者）。Host 另按设置的托盘/窗口存在门禁拦截 |
| `island.releaseScenario()` | 需 `island.scenario`；仅主人可释放，恢复用户常驻/下拉 |
| `island.getBoundTray()` | 需 `island.scenario`；返回插件 settings `openTrayKey`（`string \| null`） |
| `island.openBoundTray()` | 需 `island.scenario`；左键绑定托盘以打开对应应用（未绑定 / 托盘不在则报错） |
| `island.onBarClick(cb)` | Shortcuts iframe：岛栏摘要被点击时回调（无 panel 也可） |
| `panel.openSession()` | 需 `island.panel`；临时展开该插件面板 |
| `panel.closeSession()` | 清会话并收起 |
| `panel.close()` | 面板内关闭 |

DnD → `island.drop` 赢家。`excludeFromPullContent: true` → 不进下拉内容列表。`island.scenario` 插件自动不进常驻/下拉列表。

## `hub.shortcuts.*`（快捷区 iframe）

| 方法 | 说明 |
|------|------|
| `getBounds()` | `{ height, barHeight, width, maxExpandWidth }`；高度恒为 **28** |
| `requestSize({ width })` | 通知 Host 条宽 |
| `setBadge` | 仅无网页入口的回退 chip |
| ~~`setPins` / `clearPins`~~ | **废弃** |

## `hub.popup.*`

`open_plugin_popup` / `close_plugin_popup`；注入后读 `popup.css` / `popup.js`。快捷区可开弹窗；岛面板 **故意** 不注入 `popup.open`。

默认尺寸 **320×480**。插件可通过 `settings` 声明 `popupWidth` / `popupHeight`（Host 读取并 clamp：宽 280–720、高 320–900）；亦可 `hub.popup.open({ width, height })` 覆盖。

## `hub.everything.*`（需 `everything.search`）

经 Host 内置 Everything SDK（`Everything64.dll` IPC）查询本机索引。**需本机 Everything 客户端正在运行。**

| 方法 | 说明 |
|------|------|
| `status()` | `{ available, running, dbLoaded, version?, error? }` |
| `search(query, opts?)` | `opts`: `max` / `offset` / `matchCase` / `matchWholeWord` / `matchPath` / `regex` / `pathPrefix` → `{ query, total, results[] }` |
| `open(path)` | `ShellExecute` 打开文件或文件夹 |
| `reveal(path)` | 资源管理器 `/select` 定位 |

`results[]`：`{ name, path, fullPath, isFolder, isFile, size? }`。

## `hub.sysmon.*`（需 `system.monitor`）

基座 Rust 采样本机状态（`sysinfo` + 可选 NVIDIA NVML）。

| 方法 | 说明 |
|------|------|
| `snapshot()` | `{ cpu, memory, disks[], temperatures[], cpuTempC?, gpuTempC?, effectiveTempC?, updatedAtMs }` |

`effectiveTempC` = `max(cpuTempC, gpuTempC)`（仅有一侧时用该侧）。Windows 温度为 best-effort；无传感器时字段为 `null`。

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



