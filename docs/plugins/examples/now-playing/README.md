# 正在播放 `com.window-hub.now-playing`

对接本机 [Now Playing](https://github.com/Widdit/now-playing-service) HTTP API。

## 表面

| 表面 | 行为 |
|------|------|
| 快捷区 | **不显示**（`manage: none` + `requestSize(0)` 隐形 worker） |
| 岛栏常驻 | 当前歌词 / 歌名（`setBar`）；需在设置里选为「岛栏常驻」 |
| 岛下拉 | 迷你播放器；需在设置里选为「下拉内容」或点岛栏打开会话 |

设置在 **插件市场 / 本插件设置**（`apiBase`、`pollMs`、`barMode` 等）。

## API（本机默认 `http://127.0.0.1:9863`）

来自软件内「API 接口」页 / 源码 `NowPlayingController` / `LyricController`：

- `GET /api/query` — 歌曲 + 播放器
- `GET /api/query/track` / `player` / `progress` / `hasSong` / `isConnected`
- `GET /api/lyric` — 歌词（前端按时轴解析）
- 桌面端 `GET http://127.0.0.1:9864/player/show` — 打开自带播放器窗

播放 / 暂停 / 切歌：Now Playing **无遥控接口**，下拉按钮走 Host `media.keys`（系统媒体键，对多数播放器有效）。

## 安装

设置 → 插件 → **导入示例：正在播放**，或开发目录指向本文件夹。需本机已运行 Now Playing。
