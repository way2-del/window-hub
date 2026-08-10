# 天气（官方插件）

岛栏常驻摘要（`hub.island.setBar`）+ 下拉详情。凭证/刷新间隔在 **插件设置**（`settings[]`），数据缓存 `hub.storage` 键 `cache`。

快捷区入口为隐形 worker（宽 1px），仅负责轮询，不占视觉条。
