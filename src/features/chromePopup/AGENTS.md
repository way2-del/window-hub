# 宿主 Chrome 弹窗壳

Wi‑Fi / 托盘收纳 / 控制中心 / 输入法 等顶栏飞出菜单共用 `ChromePopupShell`。

- 组件：`ChromePopupShell.tsx`
- 样式：`chromePopupShell.css`（DWM 负责外圆角，壳铺满 HWND）
- 测高选择器：`CHROME_POPUP_SHELL_SELECTOR`（配合 `popupFit`，最高屏幕 2/3）

新加同类弹窗：包一层 `ChromePopupShell`，用 `CHROME_POPUP_SHELL_SELECTOR` 做 `schedulePopupFit`，不要再复制 `.xxx-popup-shell` 背景/圆角。

短内容默认 `overflow: hidden`（无滚动条）；只有 `fitPopupToContent` 触顶 2/3 屏时才会加上 `is-scrollable`。
