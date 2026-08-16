# 文件搜索（Everything）

官方插件：经 Host `hub.everything.*` 搜索本机文件。**仅灵动岛情景临时**，不进快捷区。

## 情景临时（Alt+空格）

与「正在播放」同类：**`island.scenario`** 暂代岛栏 + 下拉，**不改**常驻/下拉 prefs。

| 操作 | 效果 |
|------|------|
| **Alt+空格** | `claimScenario` → **仅**激活岛栏搜索框（不立刻下拉） |
| **回车 / 点「搜索」** | 展开下拉；空关键字 → 文件夹+历史；有字 → 结果列表 |
| **再按 Alt+空格 / Esc** | 退出搜索情景，归还常驻 |

设置 → 情景临时 列表可见本插件；可配存在门禁（默认无门禁即始终可 claim）。

## 依赖

- 本机已安装并**正在运行** [Everything](https://www.voidtools.com/)
- Host 已内置 `Everything64.dll`

## 能力 / 槽位

| 项 | 用途 |
|----|------|
| `island.scenario` + `island.bar` + `island.panel` | 情景临时接管 |
| `everything.search` | 搜索 / 打开 / 定位 |
| `storage` | 文件夹分类与历史 |
