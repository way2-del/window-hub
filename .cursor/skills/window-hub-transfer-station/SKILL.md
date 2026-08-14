---
name: window-hub-transfer-station
description: >-
  Official Transfer Station example — shortcuts + popup, hub.staging,
  pickFiles / drag-in popup. Install via .whpx / 开发目录 / 导入示例.
  Not Host-embedded. Not island.drop.
---

# 中转站插件（示例）

**ID：** `com.window-hub.transfer-station`  
**真源：** `docs/plugins/examples/transfer-station/`（资源镜像 `src-tauri/resources/plugins/transfer-station/`）  
**非** Host 内嵌；启动不刷盘。

## 架构（v2：左侧快捷区）

| 槽位 | 作用 |
|------|------|
| `shortcuts` | 左侧 chip；点击 `hub.popup.open` |
| `entry.popup` | 弹窗内暂存预览 / 添加文件 / 拖入 / 多选批量 |

**不再**使用 `island.drop` / `island.panel` / `island.bar`（透明岛拖放不可靠）。

入口：`entry.shortcuts` + `entry.popup` → 固定 `shortcuts.*` / `popup.css` / `popup.js`。

## 安装

设置 → 插件：导入示例「中转站」/ 添加开发目录 / 安装 `.whpx`。

## 数据

- 索引：`hub.staging.*` → `plugin_kv.__staging_items`
- 载荷：`%APPDATA%/window-hub/plugins/<id>/staging/`

## 拖入文件

- **拖到快捷区图标上松开**：Host 入库并打开弹窗（正确用法）
- **系统拖文件开始**（Explorer `SysDragImage`）：Host 在**同一主窗**内露出 mini 落点（抬高顶栏 HWND，**禁止**新建弹窗 HWND）；松手入库后再 `open_plugin_popup`
- **禁止**在拖拽过程中悬停即开弹窗：新 HWND 插入拖拽会话会导致目标区禁止光标、无法放下
- 弹窗已打开时，可再往弹窗虚线区拖入（Tauri `onDragDropEvent`）

## Manifest（要点）

```json
{
  "entry": { "shortcuts": "shortcuts.html", "popup": "popup.html" },
  "slots": {
    "shortcuts": { "order": 20, "action": "popup.open", "label": "中转站" }
  },
  "capabilities": ["shortcuts", "popup", "staging", "storage"]
}
```

## 相关

- 总流程：`window-hub-plugin`
- 快捷区：`window-hub-shortcuts`
- 弹窗几何：`window-hub-plugin-popup`
