---
name: window-hub-plugin-popup
description: >-
  Window Hub 插件托管弹窗几何 — shell 边距/间距 token、Host #app.wg-shell 挂载约定、
  对照窗口组。改 entry.popup / popup.css 或新写弹窗插件时必读。
---

# 插件弹窗几何（entry.popup）

托管弹窗由 Host 打开（`open_plugin_popup`），**不渲染插件的 `popup.html` 外壳**；只注入同目录 `popup.css` + `popup.js`，挂到 Host 提供的节点上。

## Host 挂载约定（易踩坑）

```tsx
// PluginPopupHost.tsx
<main id="app" className="wg-shell" />
```

| 事实 | 含义 |
|------|------|
| 根节点固定 `#app.wg-shell` | 插件 CSS **必须**能命中 `#app` 或 `.wg-shell` |
| `popup.html` 的 class 不会出现在 DOM | 只写 `.memo-shell` 而 Host 是 `.wg-shell` → **边距全部失效、内容贴边** |
| 推荐 | 壳样式写 `#app.wg-shell`，或在 JS 里 `document.getElementById("app").className = "…"` |

真源对照：`docs/plugins/examples/window-groups/popup.css` → `.wg-shell`。

## 外边距 / 间距 token（对齐窗口组）

| Token | 值 | 用途 |
|-------|-----|------|
| `--wh-popup-pad-x` | **12px** | 左右内边距 |
| `--wh-popup-pad-top` | **12px** | 顶内边距 |
| `--wh-popup-pad-bottom` | **14px** | 底内边距（略大于顶，视觉落底） |
| `--wh-popup-gap` | **10px** | 壳内主区块垂直间距（header / 表单 / 列表） |

```css
#app.wg-shell {
  box-sizing: border-box;
  height: 100vh;
  padding: var(--wh-popup-pad-top, 12px) var(--wh-popup-pad-x, 12px)
    var(--wh-popup-pad-bottom, 14px);
  display: flex;
  flex-direction: column;
  gap: var(--wh-popup-gap, 10px);
  overflow: hidden;
}
```

### 硬性规则

1. **禁止**内容贴窗边：壳上必须有上表 padding（可与 Host 默认叠加，勿写成 0）。
2. **禁止**只给自定义 class 写 padding 却不命中 `#app` / `.wg-shell`。
3. 列表区 `flex: 1; min-height: 0; overflow: auto`，勿让列表撑破壳导致底边被裁。
4. 左右对称；底 ≥ 顶（默认 14 / 12）。
5. 区块间距用 `gap: 10px`，不要靠负 margin 顶边。

## 壳内次级间距（建议）

| 元素 | 建议 |
|------|------|
| 标题 | 13px / weight 650 |
| 辅助说明 | 11px、muted |
| 主按钮高 | 28–32px |
| 列表行内边距 | ≈ 9–10px |
| 表单控件圆角 | 6–8px |

窗口尺寸默认 Host **320×480**（`PLUGIN_POPUP_W/H`）；插件可用 `settings.popupWidth` / `popupHeight` 或 `hub.popup.open({ width, height })` 自定义（clamp：宽 **280–2400**、高 **320–1600**）。内容按窗宽布局，勿假定固定像素。

大画布类插件可额外使用：

| API | 说明 |
|-----|------|
| `hub.popup.open({ nativeFrame, resizable, windowedFullscreen })` | `nativeFrame: true` → 系统标题栏（同设置窗）；最大化用系统按钮或 `windowedFullscreen` |
| `hub.popup.setWindowedFullscreen(bool)` | 原生窗 maximize；无边框则铺满工作区 |
| `hub.popup.resize({ width, height })` | 调整已开弹窗 |

官方示例：`docs/plugins/examples/excalidraw/`（始终 `nativeFrame: true`）。

## 自检

- [ ] 打开弹窗后，四边可见空隙（约 12px）
- [ ] 开发者工具里 `#app` 的 computed padding 非 0
- [ ] class 命中 `.wg-shell` 或已在 JS 重设 class
- [ ] 无 `alert` / `confirm` / `prompt`

## Related

- 总流程：`window-hub-plugin`
- 官方对照：`window-hub-window-groups`
