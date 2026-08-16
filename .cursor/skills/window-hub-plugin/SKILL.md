---
name: window-hub-plugin
description: >-
  Window Hub 插件开发完整流程 — plugin.json slots/capabilities、三种入口
  (shortcuts/panel/popup)、hub.* 分表面矩阵、settings、.whpx 安装、窗口组与中转站示例。
  Use when creating/reviewing plugins, scaffolding .whpx packages, or extending CapGate.
---

# Window Hub 插件开发（AI 跑通全流程）

## When to use

- 从零新建插件 / 改 `plugin.json` / 对照示例改槽位
- 安装、打包、开发目录热更
- 扩展 Host API / CapGate（先查「勿臆造」）

**先读本 skill，再按需打开子 skill**（快捷区 / 岛面板 / 岛通知 / 沉浸 / 窗口组 / 中转站）。

## 一句话模型

```
插件 = 静态包（HTML/CSS/JS）+ plugin.json
系统能力 = 只经 window.hub.*（CapGate）
缺能力 = Companion 独立进程脚本（非 DLL 进主进程）
```

- **禁止** `alert` / `confirm` / `prompt` → 应用内 UI
- **禁止** Host 硬编码插件 id；槽位靠 `slots` + `capabilities` 解析
- **禁止** 自写 `%APPDATA%` / 宿主 `localStorage` → 只用 `hub.storage` / `hub.settings` / `hub.staging`

## 表面与槽位

| 表面 | Slot | 入口文件 | 典型能力 |
|------|------|----------|----------|
| 快捷区（状态菜单左侧） | `shortcuts` | `entry.shortcuts` → iframe 自画 | `shortcuts` + 常用 `storage`/`popup`/`windows.*` |
| 灵动岛下拉面板 | `island.panel` | `entry.panel` | `island.panel` |
| 岛栏摘要 | `island.bar` | （无独立入口；API / Host 同步） | **capability + slot** `island.bar`；全局设置「岛栏常驻」竞选；`excludeFromBarResident` 仅临时条；**`adaptiveWidth`** 折叠岛宽随文案（见下） |
| 情景临时 | `island.scenario` | （无独立入口） | 与 bar+panel 同用；`claimScenario`/`releaseScenario` 暂代岛栏+下拉，不改 prefs；Host 配存在门禁；`settings.openTrayKey` → `openBoundTray` |
| 岛上拖放 | `island.drop` | （Host DnD） | **必须** `island.drop` + `staging` |
| 岛通知横幅 | `island.notify` | （无独立 iframe；`hub.notify`） | `notify` |
| 托管弹窗 | — | `entry.popup` | `popup` |

**命名：** 顶栏左区叫**快捷区**，勿称 Dock。Dock / Widget 为后续表面，**schema 尚无对应 slot**。

## AI 端到端流程（按序执行）

### 0. 选型（对照两个官方示例）

| 需求 | 跟谁学 | 目录 |
|------|--------|------|
| 快捷区常驻条 + 弹窗管理 | **窗口组** | `docs/plugins/examples/window-groups/`（`manage=custom`） |
| 快捷区入口 + 可自建应用库 | **应用库** | `docs/plugins/examples/app-library/`（`manage=custom`） |
| Everything 文件搜索 | **文件搜索** | `docs/plugins/examples/file-search/`（仅 `island.scenario` + Alt+空格；无快捷区；`everything.search`） |
| 拖放暂存 + 岛栏 + 矮面板 | **中转站** | `docs/plugins/examples/transfer-station/` |
| 岛栏摘要 + 下拉详情 + settings | **天气** | `docs/plugins/examples/weather/`（shortcuts `manage=none`） |
| 岛栏歌词自适应宽 + 迷你播放器 | **正在播放** | `docs/plugins/examples/now-playing/`（`island.scenario` + `adaptiveWidth`；健康 claim） |
| 快捷区成语 chip + 历史弹窗 + 带调拼音 | **成语** | `docs/plugins/examples/idiom/`（`manage=custom`；`pinyin-pro.min.js`） |
| 仅下拉面板（摄像头等） | **镜子** | `docs/plugins/examples/mirror/` |

打包资源镜像：`src-tauri/resources/plugins/{window-groups|app-library|transfer-station|weather|mirror|idiom|now-playing}/`（与 docs 示例保持同步）。

### 1. 建目录与 `plugin.json`

```
my-plugin/
  plugin.json
  # 按需：
  shortcuts.html + shortcuts.css + shortcuts.js   # 快捷区（stem 同名）
  panel.html     + panel.css     + panel.js       # 岛面板（Host 固定读这两个文件名）
  popup.html     + popup.css     + popup.js       # 弹窗（Host 固定读这两个文件名）
  icon.svg / logo.png
```

**硬约定（否则空白）：**

- Panel：Host **只**注入 `panel.css` / `panel.js`（与 `entry.panel` 旁路同目录），**不要**只写 `index.html` 指望自动找脚本。
- Popup：注入 `popup.css` / `popup.js`（Host 挂 `#app.wg-shell`，**不**用 popup.html 外壳）。边距见 `window-hub-plugin-popup`。
- Shortcuts：按 entry 文件名 stem 找 `{stem}.css` / `{stem}.js`。

`id`：小写 + `.` / `-`，须含 `.`（如 `com.example.foo`）。

### 2. 填 slots + capabilities（最小权限）

```json
{
  "id": "com.example.demo",
  "name": "Demo",
  "version": "1.0.0",
  "official": false,
  "entry": {
    "shortcuts": "shortcuts.html",
    "popup": "popup.html",
    "panel": "panel.html"
  },
  "slots": {
    "shortcuts": {
      "icon": "icon.svg",
      "label": "Demo",
      "order": 40,
      "action": "popup.open",
      "manage": "custom"
    },
    "island.panel": {
      "defaultSize": { "w": 380, "h": 180 },
      "excludeFromPullContent": false
    }
  },
  "settings": [],
  "capabilities": ["storage", "shortcuts", "popup", "island.panel"]
}
```

| Capability | 真实可调 API | 安装敏感？ |
|------------|--------------|------------|
| `storage` | `hub.storage.*`、`hub.settings.*` | 否 |
| `shortcuts` | badge（回退 chip）；条内几何见下 | 否 |
| `popup` | `hub.popup.open` / Host `open_plugin_popup` | 否 |
| `island.panel` | `hub.panel.openSession` / `closeSession` | 否 |
| `island.drop` | 仅参与 Host DnD 路由（无单独 hub 方法） | 建议明示 |
| `staging` | `hub.staging.*` | 是 |
| `windows.read` | `hub.windows.list/get/subscribe` | 否 |
| `windows.focus` | `hub.windows.focus` | **是** |
| `media.keys` | `hub.media.sendKey(action)` | **是** |
| `notify` | `hub.notify`（需 slot `island.notify`） | 是 |
| `network` | `hub.fetch(url, opts?)` + `permissions.network` 白名单 | 是 |
| `clipboard.*` | **仅 schema 占位，无实现** — 勿调用 | 是（若声明） |
| `everything.search` | `hub.everything.status/search/open/reveal`（需本机 Everything） | **是** |

**槽位门控（非 capability）：** `hub.island.setBar` / `clearBar` 需 `slots["island.bar"]`。`claimScenario` / `releaseScenario` 需 `slots["island.scenario"]`（且 bar+panel）。

**岛栏宽自适应（基座能力）：** 在 `slots["island.bar"]` 声明即可，**无需**新 capability：

```json
"island.bar": {
  "order": 10,
  "adaptiveWidth": true,
  "minWidth": 260,
  "maxWidth": 520
}
```

| 字段 | 含义 |
|------|------|
| `adaptiveWidth` | `true` → Host 按 `setBar` 文案测量并调整**折叠态**岛宽 |
| `minWidth` / `maxWidth` | 逻辑像素；默认约 220–560；超长仍 ellipsis |
| 未声明 | 折叠宽固定约 300，文案 ellipsis |

插件只需正常 `hub.island.setBar({ text, title? })`；**禁止**自算宽度改壳。官方示例：正在播放 `com.window-hub.now-playing`。

### 3. 按表面写 UI（`window.hub` 分表面矩阵）

同一插件在不同入口注入的 API **不一致**，按表选用：

| API | Popup（Tauri 壳） | Panel iframe | Shortcuts iframe |
|-----|-------------------|--------------|------------------|
| `storage` / `settings` | ✅ | ✅ | ✅ |
| `windows.*` | ✅ | ✅ | ✅ |
| `staging.*` | ✅ | ✅ | ❌ |
| `island.setBar/clearBar` | ✅ | ✅ | ✅（需 `island.bar`；备忘类勿常驻写栏） |
| `panel.openSession/closeSession/close` | ✅ | ✅ | ❌ |
| `popup.open/close` | close ✅ | ❌ | ✅ open/close |
| `shortcuts.getBounds/requestSize` | ❌ | ❌ | ✅ |
| `shortcuts.showTip/hideTip` | ❌ | ❌ | ✅（Host tip；与插件弹窗同 Mica） |
| `shortcuts.setBadge` | ✅ | ❌ | ❌ |
| `foreground.subscribe` | ❌ | ❌ | ✅ |
| `notify` | ✅ | ✅ | ✅ |
| `fetch` | ✅ | ✅ | ✅ |

- 快捷区细则 → `window-hub-shortcuts`（高度 `getBounds().height` / `--wh-bar-h`，禁止写死 28；**`slots.shortcuts.manage`**：`custom` 自画弹窗 / `none` 不显示 / `settings` Host 跳转设置；**hover** 用 `hub.shortcuts.showTip` 或 `title`（自动改 Host tip），禁止依赖系统原生气泡；**布局**：按住 Ctrl + 左键拖动排序，持久化 `prefs_shortcuts.pluginOrder`）
- 岛面板尺寸 → `settings.panelWidth`/`panelHeight` → 否则 `defaultSize` → 否则 380×220；`excludeFromPullContent` 不进下拉列表
- 拖放赢家：`order` 最低且同时具备 `island.drop` + `staging`
- **废弃：** `hub.shortcuts.setPins` / `clearPins`（勿用）

### 4. 声明式设置（可选）

`plugin.json` → `settings[]`（`boolean|string|number|select|radio|multiSelect`）  
值：`plugin_kv.__settings`，运行时 `hub.settings.*`（需 `storage`）。  
Host 设置页自动渲染；中转站：`panelWidth` / `panelHeight`；快捷区弹窗：`popupWidth` / `popupHeight`（成语等）。

### 5. 持久化

| Key | 谁写 |
|-----|------|
| 普通键（如 `store`） | `hub.storage.*` |
| `__settings` | `hub.settings.*` |
| `__staging_items` | 仅 `hub.staging.*` |
| `__shortcuts_pins` | 废弃，勿写 |

唯一库：`%APPDATA%/window-hub/window-hub.db` → 表 `plugin_kv`。详见 `docs/plugins/local-storage.md`。

### 6. 安装与验证

1. **开发目录**：设置 → 插件 →「添加开发目录」→ 安装前确认表面/能力 → id 变为 `{id}__dev`
2. **示例导入**：预览 `preview_example_plugin` → 确认 → `install_example_plugin`
3. **打包**：`pack_plugin_directory` → `.whpx` → 预览后安装
4. 启用后看：`plugins-changed`、快捷区条、岛下拉、拖放、设置项

安装确认弹层展示：快捷区（是否弹窗）、岛通知、岛下拉、岛栏/拖放等 **surfaces** + **capabilities** + `permissions.network` 主机列表。

**无**启动刷盘 / `include_str` 内嵌业务插件。

### 7. 自检清单（合并前勾完）

- [ ] `plugin.json` id / slots / capabilities / entry 与真实文件一致
- [ ] panel 旁有 `panel.css`+`panel.js`（若有 panel）；popup / shortcuts 同理
- [ ] 未使用 `setPins`、`alert`/`confirm`/`prompt`
- [ ] 未调用未实现的 `hub.clipboard.*`；`hub.fetch` 已声明 `network` + `permissions.network`；`hub.notify` 已声明 `notify` + `island.notify`
- [ ] 快捷区高度来自 `getBounds` / `--wh-bar-h`
- [ ] 长摘要 / 歌词：`slots["island.bar"].adaptiveWidth: true`（可选 min/max）；**勿**自改岛壳宽
- [ ] 无硬编码官方插件 id 的 Host 后门依赖
- [ ] 敏感 capability 在 README / 安装说明中写明
- [ ] 缺系统能力 → Companion 脚本路径（`docs/plugins/companion-scripts.md`），非塞 DLL

## Manifest 完整参考

Schema：`docs/plugins/plugin.schema.json`  
API：`docs/plugins/host-api.md` · SDK：`docs/plugins/sdk.md`  
市场/安装：`docs/plugins/marketplace.md`

### 窗口组（快捷区 + popup）

```json
{
  "id": "com.window-hub.window-groups",
  "entry": { "popup": "popup.html", "shortcuts": "shortcuts.html" },
  "slots": {
    "shortcuts": {
      "icon": "icon.svg",
      "label": "窗口组",
      "order": 20,
      "action": "popup.open"
    }
  },
}
```

数据：`hub.storage` 键 `store`（含 `pins`）。细则：`window-hub-window-groups`。

### 中转站（drop + bar + panel）

```json
{
  "id": "com.window-hub.transfer-station",
  "entry": { "panel": "panel.html" },
  "slots": {
    "island.bar": { "order": 10 },
    "island.drop": { "order": 10 },
    "island.panel": {
      "defaultSize": { "w": 560, "h": 152 },
      "excludeFromPullContent": true
    }
  },
  "settings": [
    { "key": "panelWidth", "type": "select", "default": 560, "options": [] },
    { "key": "panelHeight", "type": "select", "default": 152, "options": [] }
  ],
  "capabilities": ["island.panel", "island.drop", "staging", "storage"]
}
```

细则：`window-hub-island-panel` + 示例目录 `transfer-station/`。

## 勿臆造（当前未实现）

写插件时 **不要**假设已有：

- `hub.clipboard.*`
- `entry.development` 热链到 `localhost` Vite（仅 `__dev` 目录安装）
- Dock / Widget slot
- 远程市场 / 签名 `.whpx` / CLI publish
- 统一三表面完全相同的 `hub` 全集（快捷区可 popup；岛/面板不能开弹窗 — **有意不同**）
- 通知按钮自定义坐标（仅允许 `slot: start|end`，见 island-notify skill）

缺能力 → 提 CapGate 提案，或 Companion 本地脚本。

## Related skills

- [window-hub-shortcuts](../window-hub-shortcuts/SKILL.md)
- [window-hub-plugin-popup](../window-hub-plugin-popup/SKILL.md)
- [window-hub-island-panel](../window-hub-island-panel/SKILL.md)
- [window-hub-island-notify](../window-hub-island-notify/SKILL.md)
- [window-hub-island-immerse](../window-hub-island-immerse/SKILL.md)
- [window-hub-window-groups](../window-hub-window-groups/SKILL.md)
- [window-hub-transfer-station](../window-hub-transfer-station/SKILL.md)

