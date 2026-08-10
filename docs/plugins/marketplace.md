# 插件市场与发布阶段

## 阶段

| 阶段 | 状态 |
|------|------|
| P0–P4 规范 / 本地运行时 / 官方示例 / `.whpx` | **已落地** |
| P5 远程市场 / 签名 | 草图，未实现 |
| P6 CLI create/publish | 草图，未实现 |

## 本地安装（当前）

1. 设置 → **插件** →「安装 .whpx」或「添加开发目录」或「导入示例」
2. **安装前预览**（`preview_plugin_from_path` / `preview_example_plugin`）：展示所用界面表面（快捷区是否弹窗、岛通知、岛下拉、岛栏/拖放等）、capabilities、`permissions.network` 主机列表；用户确认后才安装
3. 校验 `plugin.json`（id/name/version + slots|entry）
4. 解压到 `%APPDATA%/window-hub/plugins/{id}/`，写 `registry.json`
5. `plugins-changed` → 前端热加载
6. 开发目录安装：id 自动加 `__dev`

首次启动**不会**自动安装示例。导入：

- `preview_example_plugin` → 确认 → `install_example_plugin("window-groups"|"transfer-station")`

源：`docs/plugins/examples/{name}` 或打包资源 `src-tauri/resources/plugins/{name}`。  
`manifest.official` 仅 UI 徽章。

打包：`pack_plugin_directory` → `.whpx`。

## registry 示意

```json
{
  "plugins": [
    {
      "id": "com.window-hub.window-groups",
      "name": "窗口组",
      "version": "1.2.4",
      "path": "...",
      "enabled": true,
      "isDev": false,
      "capabilities": ["shortcuts", "storage", "popup", "windows.read", "windows.focus"],
      "manifest": {}
    }
  ]
}
```

安装确认弹层与已安装列表均展示 capabilities；敏感项高亮。

## `.whpx`

- zip；根目录必须含 `plugin.json`
- 无运行时 `node_modules`（构建产物打进包）
- 禁止未审核原生二进制

## 市场（P5，未实现）

发现 / 下载 / 更新 / ed25519 签名 / kill-switch — 仅规划。

## Companion 上架注意

附带脚本**源码模板**即可；运行态由用户本机安装，Hub 设置页登记自启。见 [companion-scripts.md](./companion-scripts.md)。
