# 待办（官方示例）

灵动岛下拉面板管理待办；**新建时记录创建日期与时间**；岛栏显示未完成数量。

## 表面

| 槽位 | 行为 |
|------|------|
| `shortcuts` | 隐形 worker：冷启动与 storage 变更时同步岛栏 |
| `island.panel` | 列表 / 添加 / 完成 / 删除；每条显示「创建 YYYY-MM-DD HH:mm」 |
| `island.bar` | 有未完成时显示「待办 N」（默认优先级在歌词之后、天气之前） |

数据键 `store`（`hub.storage`）。首次启动由 `ensure_official_plugins` 安装。
