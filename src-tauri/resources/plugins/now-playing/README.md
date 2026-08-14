# 正在播放 `com.window-hub.now-playing`

对接本机 [Now Playing](https://github.com/Widdit/now-playing-service) HTTP API。

## 表面

| 表面 | 行为 |
|------|------|
| 快捷区 | **不显示**（`manage: none` + `requestSize(0)` 隐形 worker） |
| 岛栏常驻 | 当前歌词 / 歌名（`setBar`）；需在设置里选为「岛栏常驻」；`adaptiveWidth` 折叠岛随歌词变宽 |
| 岛下拉 | 迷你播放器；视觉对齐官方 **iOS 歌曲组件**（`Assets/PublicExample`）；右下角「打开应用」→ `hub.island.openBoundTray`（设置 → 情景临时 → 绑定托盘） |
| 情景临时 | 健康有曲时 claim；Host 存在门禁 + 打开托盘绑定在全局设置「情景临时」二级页 |

设置在 **插件市场 / 本插件已安装详情**（`apiBase`、`pollMs`、`barMode` 等插件 settings + Host 情景门禁 / 打开托盘绑定）。全局设置「情景临时」仅跳转到该详情页。

## 下拉 Panel 素材来源

官方示例（与截图同款布局）：

- 仓库：[Widdit/now-playing-service](https://github.com/Widdit/now-playing-service)
- 路径：[`Assets/PublicExample/`](https://github.com/Widdit/now-playing-service/tree/master/Assets/PublicExample)
  - `index.html` / `style.css` / `main.js`
  - `assets/icon_{rewind,play,pause,forward}.svg`（控件图标）
  - 右侧频谱：封面纹理裁切进 canvas 竖条（见 `main.js` `drawWaveform`）

本插件：`panel.html` / `panel.css` / `panel.js`（控件 SVG **内联**进 HTML——Host 用 `srcdoc` 注入面板，相对路径 `./assets/*` 无法加载）。`assets/*.svg` 与 `vendor/public-example/` 仅作对照副本。

完整播放器页在前端仓：[now-playing-frontend](https://github.com/Widdit/now-playing-frontend) 的 `Player.tsx`（偏 AMLL 全屏，不是这张迷你卡）。

## API（本机默认 `http://127.0.0.1:9863`）

- `GET /api/query` — 歌曲 + 播放器
- `POST /api/cover/convert` — 封面转 base64（可选）
- 媒体键：`hub.media.sendKey(play_pause|next|previous)`
