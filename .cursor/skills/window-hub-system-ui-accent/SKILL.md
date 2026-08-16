---
name: window-hub-system-ui-accent
description: >-
  Window Hub 系统 UI 强调色 — --sys-accent 绿色（同输入法）、WLAN/IME/设置/弹窗。
  改 Host chrome、WLAN、输入法或系统开关配色时必读。
---

# 系统 UI 强调色（绿色）

源码 token：`src/App.css`、`src/settings.css` 的 `:root` → `--sys-accent*`。

规范全文：`.cursor/rules/system-ui-accent.mdc`。

## 唯一强调色

系统自绘 UI **不要用蓝色**。对齐输入法方块绿色：

- 主色 `--sys-accent` = `#34c759`
- 软色 `--sys-accent-soft` = `#3dd68c`（IME mark）
- 按下/浅色 `--sys-accent-press` = `#30d158`

WLAN 的 `--wifi-accent` **必须** `var(--sys-accent)`，认证弹窗主按钮同色。

## 改色时

1. 只改 `:root` token，勿在各组件散落硬编码绿/蓝
2. 插件内容区品牌色除外；Host 边框/开关/选中仍跟 token
3. 同步检查 `WifiPopupApp`、`WifiAuthPopupApp`、`ilang-*`、`pref-switch`、`scenario-tray-picker`
