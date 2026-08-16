---
name: window-hub-status-menu
description: >-
  Window Hub 状态/Dock 右键菜单列表几何 — .status-menu-shell token、贴合内容高度、
  单选项垂直居中、禁止顶空。改 StatusMenu.css / StatusMenuPopupApp / fitPopup 时必读。
---

# 状态菜单列表规范（status-menu）

独立 HWND 弹窗（`status-menu-popup`），壳类名 `.status-menu-shell`。真源：`src/components/StatusMenu.css`、`src/StatusMenuPopupApp.tsx`、`src/popupFit.ts`。

与插件弹窗（`window-hub-plugin-popup`）不同：本菜单是**短列表**，HWND **必须贴合内容高度**，不能用大 `minHeight` 撑窗。

## 几何 token（CSS 变量）

| Token | 值 | 用途 |
|-------|-----|------|
| `--sm-pad-y` / `--sm-pad-x` | **6px** | 壳四边内边距（上下对称） |
| `--sm-item-pad-y` | **8px** | 菜单行上下 padding |
| `--sm-item-pad-x` | **14px** | 菜单行左右 padding |
| `--sm-item-gap` | **2px** | 行与行之间 gap |
| `--sm-sep-margin-y` | **4px** | 分割线上下 margin |
| `--sm-sep-margin-x` | **10px** | 分割线左右 margin |
| `--sm-radius` | **10px** | 壳圆角（对齐 DWM ROUND） |
| `--sm-font-size` | **13px** | 行字号 |

```css
.status-menu-shell {
  padding: var(--sm-pad-y) var(--sm-pad-x);
  display: flex;
  flex-direction: column;
  justify-content: center; /* 残余空隙上下均分 */
  gap: var(--sm-item-gap);
}
```

## 硬性规则

1. **HWND 贴合内容**：`fitPopupToContent({ selector: ".status-menu-shell", minHeight ≤ 40 })`。  
   **禁止** `minHeight: 72`（单行菜单会顶出大块空白）。
2. **上下对称**：壳 `padding-top === padding-bottom`。禁止「底大顶小」的落底视觉（那是插件表单弹窗的规则，不适用于菜单列表）。
3. **垂直居中**：`justify-content: center`。  
   **禁止** `.is-origin-up { justify-content: flex-end }` / `.is-origin-down { flex-start }` 来「贴边塞内容」——贴边只用于开合动画的 `pinBottom`，不用于壳内排版。
4. **单选项**：行高 ≈ `2 × item-pad-y + line-height`，外加壳 `2 × pad-y`；视觉上选项应在胶囊正中，上下空隙相等且 ≤ pad-y。
5. **行不拉伸**：`.status-menu-item` / `.status-menu-sep` 使用 `flex: 0 0 auto`，禁止 `flex: 1` 把一行拉满整窗。
6. **测量**：`fitPopupToContent` 前壳保持 `height: auto` 可测；展示时 `height: 100%` 填 HWND，靠 center 消化亚像素残余。

## 开合方向 vs 排版

| 概念 | 职责 |
|------|------|
| `is-origin-up` + `pinBottom` | Dock：窗底钉在点击点上方，增高时向上长 |
| `is-origin-down` | 岛标题：窗顶钉住，向下长 |
| `justify-content: center` | **始终**；与 origin 无关 |

## 行与分割线

| 元素 | 规则 |
|------|------|
| `.status-menu-item` | block 全宽；hover 浅底；危险项 `.is-danger` |
| `.status-menu-sep` | 1px；不计作「可点行」；margin 用 token |
| 危险色 | 深色 `#ff8a80` / 浅色 `#dc2626` |

## 自检

- [ ] 仅「删除分割线」一项时：上下空隙对称，无明显顶空
- [ ] 多行菜单：行间距 2px，分割线不显得「悬空一大截」
- [ ] DevTools：shell 的 `padding-top` ≈ `padding-bottom`；无 `justify-content: flex-end`
- [ ] `fitPopupToContent` 的 `minHeight` ≤ 40

## Related

- 插件弹窗边距（表单类）：`window-hub-plugin-popup`（底 ≥ 顶，与本规范不同）
- Dock 打开入口：`DockApp.tsx` → `open_status_menu_popup`
- 岛标题入口：`StatusMenu.tsx`
