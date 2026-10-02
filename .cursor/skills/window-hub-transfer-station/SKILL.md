---
name: window-hub-transfer-station
description: >-
  Official Transfer Station example — island.drop + island.bar + island.panel,
  hub.staging, panelWidth/panelHeight settings, excludeFromPullContent.
  Install via .whpx / 开发目录 / 导入示例. Not Host-embedded.
---

# 中转站插件（示例）

**ID：** `com.window-hub.transfer-station`  
**真源：** `docs/plugins/examples/transfer-station/`（资源镜像 `src-tauri/resources/plugins/transfer-station/`）  
**非** Host 内嵌；启动不刷盘。

## 架构

| 槽位 | 作用 |
|------|------|
| `island.drop` | Host 把岛上 DnD 路由到本插件（须同时有 `staging`） |
| `island.bar` | 有内容时岛栏摘要；可 `hub.island.setBar`，Host 也会跟 `staging-changed` 同步 |
| `island.panel` | 横向缩略图面板；`excludeFromPullContent: true`（不进「下拉内容」） |

入口：仅 `entry.panel` → `panel.html` + **固定** `panel.css` / `panel.js`。

## 安装

设置 → 插件：导入示例「中转站」/ 添加开发目录 / 安装 `.whpx`。

## 数据与设置

- 索引：`hub.staging.*` → `plugin_kv.__staging_items`
- 载荷：`%APPDATA%/window-hub/plugins/<id>/staging/`
- 尺寸：拖入 / 首页页签打开时 Host 壳用仪表台 `ISLAND_HOME_PANEL_*`（640×248）；不再走独立 `panelWidth`/`panelHeight` 矮壳
## Manifest（要点）

```json
{
  "entry": { "panel": "panel.html" },
  "slots": {
    "island.bar": { "order": 10 },
    "island.drop": { "order": 10 },
    "island.panel": {
      "defaultSize": { "w": 560, "h": 152 },
      "excludeFromPullContent": true
    }
  },
  "capabilities": ["island.panel", "island.drop", "staging", "storage"]
}
```

Host：拖入或点摘要 → `openTransferHomeTab`（切中转站 tab + 仪表台尺寸）。
## 相关

- 总流程：`window-hub-plugin`
- 面板几何：`window-hub-island-panel`
- 沉浸与岛栏字色：`window-hub-island-immerse`
