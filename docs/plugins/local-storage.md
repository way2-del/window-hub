# 插件本地存储规范

宿主以单一 SQLite 库 `%APPDATA%/window-hub/window-hub.db` 持久化。

## 怎么区分

| 归属 | 落点 | 说明 |
|------|------|------|
| **宿主** | 业务表 `prefs_*` / `weather_*` / `script_launchers` / `schema_meta` | 官方设置、天气、Companion；含 `prefs_shortcuts` |
| **插件** | **唯一**表 `plugin_kv(plugin_id, key, value_json)` | **不为每个插件 / 中转站再建表** |

插件侧所有持久化（含窗口组 pins、中转站索引）都进 `plugin_kv`，用 **key** 区分 value：

| Key | 谁写 | 含义 |
|-----|------|------|
| 普通键 `store` 等 | `hub.storage.*` | 插件自己的配置/状态（窗口组 pins 真源：`store.pins`，由 shortcuts 网页画） |
| `__shortcuts_pins` | ~~setPins~~（废弃） | 旧 Host 展示缓存；新代码勿写 |
| `__staging_items` | 仅 `hub.staging.*`（宿主） | 按 pluginId 隔离的暂存索引 JSON 数组 |
| `__settings` | `hub.settings.*`（需 `storage`） | 插件 `settings[]` 声明的配置对象 |

以 `__` 开头的键为 **宿主保留**：`hub.storage` 的 get/set/remove/listKeys **不可**访问；开发者选项里仍可在 `plugin_kv` 看到。

## 必须遵守

1. **唯一入口（业务数据）**：`hub.storage.*`（需 `storage`）；禁止自写 `%APPDATA%`、禁止宿主 localStorage。
2. **Key**：`^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$`；禁止 `..` `/` `\`；禁止 `__` 前缀。
3. **Value**：JSON；单 key ≤ **512KB**；插件用户键总量建议 ≤ **8MB**。
4. **API**：`get` / `set` / `remove` / `listKeys`；无跨插件读写；`listKeys` 不返回 `__*`。
5. **官方约定**：
   - 窗口组：`store`（含 `groups` / `pins`）；快捷区用 `entry.shortcuts` 网页自画
   - 暂存：走 `hub.staging.*` → 键 `__staging_items`（按 pluginId）；弹出宽度等 → `__settings`；载荷在 `plugins/<id>/staging/`
6. **敏感数据**：勿明文存长期 token。
7. **卸载**：删该 `plugin_id` 在 `plugin_kv` 的全部行（含 `__*`）。

## 磁盘仍保留

| 路径 | 用途 |
|------|------|
| `plugins/{id}/` | 包体与静态资源 |
| `plugins/.../staging/*` | 中转站文字/图片字节（非索引） |
| `plugins/registry.json` | 安装注册（暂未迁库） |
| Startup `.cmd` | Companion 开机脚本 |

## 迁移（历史）

- schema v3+：旧 `shortcut_pins` / `staging_items` 表已迁入 `plugin_kv` 后 DROP
- schema v4：去掉 `prefs_island.staging_panel_w`（宽度在插件 `__settings`）
- schema v5：去掉宿主 `todo_items` 表
- 文件迁移标记：`schema_meta.migrated_v1`
