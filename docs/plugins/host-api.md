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
| `media.camera` | 摄像头预览；`hub.media.resetCameraPermission` / `openCameraPrivacySettings`（敏感） | ✅ |
| `notify` | `hub.notify`（需 slot `island.notify`） | ✅ Popup / Panel / Shortcuts |
| `clipboard.read` / `clipboard.write` | 剪贴板 | ❌ schema 占位，无 API |
| `network` | `hub.fetch` + `permissions.network` 白名单 | ✅ |
| `everything.search` | Everything 本机文件搜索 / 打开 / 定位（敏感；需本机 Everything 运行） | ✅ |
| `system.monitor` | `hub.sysmon.snapshot`（CPU/内存/磁盘/温度） | ✅ |
| `webview` | 托管外部网页 WebView2：打开 / 划定元素 / 后台监测（敏感） | ✅ |

**槽位门控（非 capability）：** `hub.island.setBar` / `clearBar` 需 `slots["island.bar"]`。  
设置「岛栏常驻」选中的插件可写折叠态摘要；`excludeFromBarResident` 插件（如中转站）可走临时覆盖层。  
`slots["island.scenario"]` → **情景临时**：`claimScenario` / `releaseScenario` 暂代岛栏 + 下拉（**不改** SQLite 常驻/下拉 prefs）；有此槽位的插件不出现在常驻/下拉竞选列表。
显示优先级：**中转站 overlay > 情景临时 > 岛栏常驻**；下拉：**情景 pull > panelOverride > 设置下拉内容**。折叠岛不清情景。
**左滑划掉**：折叠岛上对情景摘要左滑可清掉该层并露出常驻；Host 会抑制该插件 `setBar` 自动晋升，直到其 `clearBar`（停播/失活）或显式 `claimScenario`，避免僵尸歌词立刻抢回；健康会话在停播后再播仍可自行补回。
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

`plugin.json` → `settings[]`；类型 `boolean` / `string` / `number` / `select` / `radio` / `multiSelect` / **`hotkey`**。  
示例：中转站 `panelWidth` / `panelHeight`；文件搜索 `openSearch` / `openFavorites`（`type: "hotkey"`）。  
Host 面板尺寸：settings → `defaultSize` → 380×220。

### 全局热键（`type: "hotkey"`）

- 存规范化 chord（`Ctrl+Alt+D`）；空字符串禁用。
- 可选 `action`：命中后 Host emit `hotkey-action` `{ pluginId, action, chord }`（缺省用 settings `key`）。
- `action: "island.search.toggle"`：与系统「打开搜索」别名，不重复 `RegisterHotKey`。
- 统一页：设置 → **快捷键**（系统 / 插件）；冲突时拒绝保存。

## `hub.staging.*`

需 `staging`。事件 `staging-changed` → `{ pluginId, files, texts, images, links, total }`。  
方法：`list` / `summary` / `addText` / `addPaths` / `addImageBytes` / `remove` / `clear` / `copy` / `copyAllPaths` / `thumb` / `reveal` / `open` / `startDrag` / `subscribe`。

- `addText`：纯文本；若整段为单个 `http(s)://` URL，则存为 `kind: "link"`。
- `open(id)`：文件/图片用系统默认程序打开；链接（或文本内容为 URL）用默认浏览器打开；普通文字打开暂存 `.txt`。
- `reveal(id)`：文件/图片在资源管理器中定位；链接则打开 URL（同 `open`）。
- 岛上拖入：Host OLE 接受文件路径 + `CF_UNICODETEXT` / `UniformResourceLocator*`（浏览器选中文字与链接）；HTML5 路径仍读 `text/uri-list`（优先）与 `text/plain`，再走 `addText`。

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

默认尺寸 **320×480**。插件可通过 `settings` 声明 `popupWidth` / `popupHeight`（Host 读取并 clamp：宽 **280–2400**、高 **320–1600**）；亦可 `hub.popup.open({ width, height, resizable?, windowedFullscreen? })` 覆盖。

| 方法 | 说明 |
|------|------|
| `open(opts?)` | 打开托管弹窗；`nativeFrame: true` → **系统标题栏**（同设置窗口，可最小化/最大化/关闭）；`windowedFullscreen: true` 原生窗则 maximize，无边框则铺满工作区；`resizable: true` 可拖拽改尺寸 |
| `close()` | 关闭 |
| `resize({ width, height })` | 调整已打开弹窗尺寸（经 clamp） |
| `openAsWindow(opts?)` | 关闭当前无边框弹窗并重开为**系统标题栏**窗口（Excalidraw「窗口化」按钮） |

## `hub.media.*`

| 方法 | Capability | 说明 |
|------|------------|------|
| `sendKey(action)` | `media.keys` | `play_pause` / `next` / `previous` / `stop` |
| `prepareCamera()` | `media.camera` | 打开摄像头面板时安装 WebView2 权限回调（按需，非启动） |
| `resetCameraPermission()` | `media.camera` | 将 WebView2 相机权限重置为允许（拒绝后可再开） |
| `openCameraPrivacySettings()` | `media.camera` | 打开 Windows「隐私 → 相机」设置 |

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
  /** 点横幅中部：有则 onAction(此 id)；无则 dismiss（有 panel 则开面板） */
  defaultActionId?: string;
  actions?: {
    id: string;
    slot: "start" | "end";       // 排序用（先 start 后 end），每槽最多 1
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

需 `notify` + `slots["island.notify"]`。按钮由 Host 固定：**追加在文案后**，与图标+文案整体居中。  
**停留**：插件通知默认 `ttlMs: 0`（像微信，点开/划掉才消失）；显式传 `ttlMs` 才自动消失。多条通知可叠层显示（最多约 6 条）。  
**点横幅中部**：若设 `defaultActionId` → `onAction`；否则 dismiss（有 `island.panel` 则开面板）。细则：`.cursor/skills/window-hub-island-notify/SKILL.md`。

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

## `hub.webview`

需 capability **`webview`**（敏感）。Host 为每个插件维护独立 WebView2 用户数据目录（`%APPDATA%/window-hub/webview-profiles/{pluginId}/`），交互浏览与后台监测共用 Cookie。

```ts
hub.webview.open({ url, title? }) → { sessionId }
hub.webview.close({ sessionId })
hub.webview.navigate({ sessionId, url })
hub.webview.startPick({ sessionId }) → { selector, textPreview, outerHtml? }
hub.webview.takeLastPick() → { sessionId, selector, textPreview, outerHtml? } | null
hub.webview.snapshot({ sessionId?, url?, selector }) → { text, html? }
hub.webview.watch.start({ id, url, selector, intervalMs?, title? })
hub.webview.watch.stop({ id })
hub.webview.watch.list() → WatchInfo[]
hub.webview.onChanged(cb)   // webview-watch-changed（仅本 pluginId）
hub.webview.onScanned(cb)   // webview-watch-scanned（每次轮询，含对比字段）
hub.webview.onPick(cb)      // webview-pick-result（划定完成；弹窗可重建时用）
hub.webview.onClosed(cb)    // webview-session-closed
```

规则：仅 `http`/`https`；监测间隔钳制 30s–1h；每插件最多 20 条 watch；首次采样只建基线不通知；`watch.stop` 会清该条基线文本。禁用/卸载插件时 Host 关闭该插件全部会话与监测。划定期间 Host 会 hold 弹窗不因失焦销毁；结果也可经 `takeLastPick` / `onPick` 回填。

官方消费者：`com.window-hub.page-watch`（网页监测）。

## Companion

Hub 未提供的能力 → 独立进程脚本 + 本地 HTTP，设置页登记自启。见 [companion-scripts.md](./companion-scripts.md)。  
**禁止**未审核原生二进制进主进程。

