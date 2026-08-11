# 应用库插件 `com.window-hub.app-library`

**独立 `.whpx` 示例插件**（非 Host 内嵌）。快捷区一条入口（悬浮弹窗触发），弹窗内为可自建的应用库。

## 能力

- `shortcuts` / `storage` / `popup` / `windows.read` / `windows.focus`

## 入口

| 文件 | 用途 |
|------|------|
| `shortcuts.html` | 顶栏快捷区（库入口 + 可选固定应用） |
| `popup.html` | 应用库弹窗（增删、搜索、切换） |

快捷区高度用 `hub.shortcuts.getBounds()` / `--wh-bar-h`；**禁止** `alert` / `confirm` / `prompt`。

## 行为

- **添加**：从当前运行窗口挑选，或手填名称 + exe 关键字
- **打开**：匹配到已打开窗口则 `hub.windows.focus`；未运行时库内标「未运行」（Host 当前无启动进程 API）
- **固定**：可把常用应用钉到快捷区，一点直达

## 安装 / 导入

设置 → 插件：

1. **导入示例：应用库**
2. **添加开发目录**（本目录 → id 变为 `…__dev`）
3. **安装 .whpx**（`pack_plugin_directory` 后导入）

打包发行时同步 `src-tauri/resources/plugins/app-library/`。

## 数据

- `hub.storage` 键 `store`：`{ version, apps[] }`
- 每项：`id` / `name` / `color` / `bind{ exe, titleIncludes }` / `pinned` / `lastHwnd`

## 敏感能力

安装时会提示：`windows.focus`（切换前台窗口）。
