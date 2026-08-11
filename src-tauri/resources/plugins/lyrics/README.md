# 歌词（官方插件）

从 **网易云音乐** 读取当前曲目 / 歌词行，写入灵动岛栏（`hub.island.setBar`）。

## 用法

1. 设置 → 一般 → **岛栏常驻** → 选「歌词」
2. 打开网易云并播放；建议开启客户端 **桌面歌词**（更稳）
3. 下拉岛可看曲目与当前行

## 技术

Host `hub.media.neteaseNowPlaying`：

1. `DesktopLyrics` 窗口 + UI Automation  
2. `OrpheusBrowserHost` 标题（歌名 - 歌手）  
3. 已知版本内存指针链（兜底）

快捷区入口为隐形 worker（宽 1px），仅轮询。
