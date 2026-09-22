# 托盘瘦身重写（2026-09）

## 设计

| 旧 | 新 |
|----|----|
| 每次 NIM_* 推全量 PNG | emit **只推 meta**，PNG 按需 `get_tray_icon_glyphs` |
| publish 写 SQLite | 仅 `set_prefs` 持久化 |
| 180s resident-watchdog 风暴 | 冷启动最多 4 次 soft recover |
| 收起 sync `list_tray_icons` | 已删除 |
| 岛 morph 时仍推送 | `set_tray_ui_paused` 暂停 emit |
| slideReveal 15× setSize | 一次 fitPopupToContent |

## 开关

- 默认开：`TRAY_BOOT_ENABLED_DEFAULT` / `TRAY_UI_ENABLED`
- 紧急关：`WH_DISABLE_TRAY=1`

## 验收

1. 顶栏展开 → 收起，不 HUNG  
2. 箭头开托盘弹窗、点图标  
3. WiFi / 时间仍在顶栏右侧  
