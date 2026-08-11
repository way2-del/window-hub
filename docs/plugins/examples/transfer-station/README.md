# 中转站（官方示例）

左侧快捷区 chip；点击在**芯片下方**打开弹窗（`hub.popup.open`），不依赖中间灵动岛拖放。

## 能力

- 弹窗内「添加文件」/ 拖入文件、文字、图片
- 缩略图预览、单击复制、双击打开位置、拖出到文件夹
- 复制全部路径 / 清空
- 数据：`hub.staging.*`（`plugin_kv.__staging_items` + `staging/` 载荷）

## Manifest

| 槽位 | 作用 |
|------|------|
| `shortcuts` | 左侧 chip；`action: popup.open` |
| `entry.popup` | 弹窗 UI（`popup.css` / `popup.js`） |

版本 ≥ 2.0.0（已从岛 panel/drop 改为快捷区 + 弹窗）。

首次启动若缺失或版本落后，由 `ensure_official_plugins` 安装/升级；也可设置 → 插件 →「导入示例：中转站」。
