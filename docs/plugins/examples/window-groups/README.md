# 窗口组插件 `com.window-hub.window-groups`

**独立 `.whpx` 示例插件**（非 Host 内嵌）。宿主不自动种子、启动时不覆盖磁盘。

快捷区整条由 `entry.shortcuts` 网页自画（Host iframe 套壳）；弹窗管理组与 pins。

## 能力

- `shortcuts` / `storage` / `popup` / `windows.read` / `windows.focus`

## 入口

| 文件 | 用途 |
|------|------|
| `shortcuts.html` | 顶栏快捷区条（入口 + 固定项） |
| `popup.html` | 管理弹窗 |

**不用** `hub.shortcuts.setPins`；固定项在 `store.pins`，由 shortcuts 页渲染。

快捷区用 `hub.shortcuts.getBounds()` / `--wh-bar-h` 取状态栏高度；**全部元素垂直居中**。

## 安装 / 导入

在 Window Hub → **设置 → 插件**：

1. **导入示例：窗口组** — 从仓库 `docs/plugins/examples/window-groups`（开发）或打包资源 `resources/plugins/window-groups` 复制安装一次  
2. **添加开发目录** — 直接选本目录（安装 id 为 `…__dev`）  
3. **安装 .whpx** — 先打包再安装：

```text
pack_plugin_directory → window-groups.whpx → 安装 .whpx
```

卸载后须再次导入；宿主不会自动重装。

## 开发

修改本目录后：

1. 若用开发目录安装：改完即生效（按 Host 热加载 / 重启）  
2. 若用「导入示例」：再点一次导入会覆盖 AppData 安装副本  
3. 打包发行时同步更新 `src-tauri/resources/plugins/window-groups/`（与本目录一致）

**UI：** 禁止 `alert` / `confirm` / `prompt`；用 `.wg-confirm`。

## 数据

- `hub.storage` 键 `store`（含 `groups` / `pins`）

## 宿主职责（通用，非本插件特例）

- ShortcutsHost 挂 iframe + `shortcutsHubBridge`
- `open_plugin_popup` 注入 `window.hub`
- 设置「快捷区占用」可独占本插件
