---
name: window-hub-island-immerse
description: >-
  Window Hub 灵动岛沉浸式规则 — 唯一开关 autoImmerse、chrome 反色、与中转站共存。
  改 scheduleImmerse、岛栏配色或中转站摘要时必读。
---

# 灵动岛沉浸式（immerse）

沉浸 = 闲置后岛壳 `fill-opacity: 0.02`（勿用 0，否则 WebView2 点透、中转站拖放失效），栏内文字跟环境明暗切换（`data-chrome` = `chromeCenter.scheme`）。

源码：`src/App.tsx`（`scheduleImmerse` / `bumpIslandActivity`）、`src/App.css`（`.is-immersed`）、`src/islandPrefs.ts`（`autoImmerse`）。

## 唯一总开关

**是否沉浸只看设置「自动沉浸」**（`islandPrefs.autoImmerse`）。

- 勾选 → 闲置满 `immerseIdleSec` 后沉浸（其它瞬时阻断除外）
- 未勾选 → 永不沉浸

**禁止**为某一插件/常驻摘要（含中转站 `.has-staging`）单独永久阻断沉浸。  
否则会出现「设置开了沉浸，中转站岛却不沉浸」的不一致。

## 允许沉浸的前提

1. `autoImmerse === true`
2. 岛折叠、未拉高、托盘未开、非 busy
3. 无通知横幅、非拖放命中（`.is-drop-target`）
4. 闲置满 `immerseIdleSec`

中转站有条目时：**照常可沉浸**；摘要字色跟 `data-chrome` / `--chrome-center-fg`（与岛栏摘要同一套）。

## 瞬时阻断（可退出沉浸，不是永久例外）

| 条件 | 原因 |
|------|------|
| 展开 / `reveal` | 面板打开 |
| 托盘 / 弹窗 | 交互 |
| `.is-drop-target` | 放置高亮，需不透明壳 |
| `.is-notifying` | 横幅可读 |
| busy / pulling / springing | 动画中 |

**不要**把 `staging.total > 0` 放进 `scheduleImmerse` 早退或「强制清沉浸」的 effect。

## chrome 反色

| 状态 | `data-chrome` | 栏文字 |
|------|---------------|--------|
| 非沉浸 | `dark` | 白字 |
| 沉浸 | `chromeCenter.scheme` | `--chrome-center-fg` |
| 仅拖放命中 | 可强制 `dark` | 白字 |

中转站摘要（`.bar-staging`）在 `.is-immersed` 下须与 `.bar-weather` 一样跟 chrome，**禁止** `!important` 锁死白字 + 黑壳不透底。

## 下拉面板对比度（插件 iframe）

岛展开壳仍是黑胶囊。`island.panel`：

1. 禁止把设置窗 `data-theme=light` 灌进面板 iframe  
2. iframe / body 显式深色底 + 浅色字  
3. Host：`IslandPanelHost` 注入 `data-theme="dark"`

面板 UI 保持精简（列表 + 复制路径/删除）；勿叠大虚线放置区 + 多按钮卡片。

## 中转站数据

- 本地文件：`addPaths` **只记绝对路径**，不 `fs::copy` 缓存  
- `copy`：文件/图片 → 剪贴板路径字符串；文字 → 正文  
- `remove`/`clear`：只删 staging 自有载荷，不删用户原文件  

详见 `docs/plugins/host-api.md` → `hub.staging.*`。

## 检查清单

- [ ] 自动沉浸勾选：闲置后含中转站摘要的岛也能透底，字色跟 chrome
- [ ] 自动沉浸未勾选：永不沉浸
- [ ] 无 `staging.total` 永久阻断
- [ ] 文件条目不复制本体；按钮为「复制路径」
- [ ] 改沉浸逻辑时同步本 skill
