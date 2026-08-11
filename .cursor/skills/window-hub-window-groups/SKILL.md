---
name: window-hub-window-groups
description: >-
  Official Window Groups example plugin — store.pins, shortcuts.html iframe
  strip (no setPins), install via .whpx / 开发目录 / 导入示例. Not Host-embedded.
  Never use browser alert/confirm/prompt.
---

# 窗口组插件（示例）

**ID：** `com.window-hub.window-groups`  
**性质：** 独立可导入插件（真源：`docs/plugins/examples/window-groups`）。**不是** Host `include_str` 内嵌，启动不刷盘。

## 禁止浏览器原生弹窗

禁止 `alert` / `confirm` / `prompt`；用 `.wg-confirm`。

## 架构

| 表面 | 文件 | 说明 |
|------|------|------|
| 快捷区 | `shortcuts.html` | Host iframe；自画入口 + pins |
| 弹窗 | `popup.html` | 管理组 / 固定；只写 `store`；壳边距见 `window-hub-plugin-popup`（`.wg-shell` 12/12/14） |

**禁止** `hub.shortcuts.setPins`。popup 保存后 Host 发 refresh，shortcuts 条重读 `store`。

## 安装

设置 → 插件：

- **导入示例：窗口组**（`install_example_plugin`）
- **添加开发目录**（选本示例目录）
- **安装 .whpx**（`pack_plugin_directory` 后导入）

## 快捷区垂直对齐（必须）

状态栏高度经 `hub.shortcuts.getBounds()` / `--wh-bar-h` 取得（见 `window-hub-shortcuts`）。

**窗口组要求：条内所有元素垂直居中**（manage 图标、chip 文案、绿点、badge、分隔线）。

`slots.shortcuts.manage` 必须为 **`custom`**：插件自画 2×2 管理钮 + `hub.popup.open`；**不要**写成 `settings`（否则 Host 再画一颗会重复）。细则见 `window-hub-shortcuts`。

```css
html, body, .wg-bar, .wg-chip {
  height: var(--wh-bar-h, 28px);
  display: flex;
  align-items: center;
}
```

`shortcuts.js` boot 时调用 `applyBarGeometry()` 写入 `--wh-bar-h`；勿写死像素高度做对齐。

## 数据

```ts
type WindowGroupStore = {
  version: 2;
  activeGroupId: string | null;
  groups: WindowGroup[];
  pins: Array<{ id: string; kind: "group" | "window"; refId: string; width?: number }>;
};
```

`hub.storage` 键 `store`。

## Manifest

```json
{
  "entry": {
    "popup": "popup.html",
    "shortcuts": "shortcuts.html"
  },
  "slots": { "shortcuts": { "icon": "windows", "label": "窗口组", "order": 20, "action": "popup.open", "manage": "custom" } },
  "capabilities": ["shortcuts", "storage", "popup", "windows.read", "windows.focus"]
}
```
