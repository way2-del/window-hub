# Window Hub · 灵动桌面

**Windows 桌面软件** — 用顶栏和底部 Dock 重新组织日常操作；中间是可定制的灵动岛，旁边是可插拔的快捷区。产品名也叫 **灵动桌面**。

> Windows 11（主要适配）· Windows 10 待测试 · v0.2 · [Tauri 2](https://tauri.app/) + React 19 + Rust / Win32  
> [English](#english)

## 这是什么

灵动桌面是一层贴在 Windows 上的 **桌面壳**：顶部的顶栏、底部的 Dock，都可以代替系统原本的任务栏（占位工作区、聚合托盘与快捷入口）。

布局上认同 **macOS 那套「窄顶栏 + Dock」**：顶栏常驻、占高很小，日常切换不抢屏；Dock 管启动与切换。Windows 自带任务栏偏厚、偏「一栏扛所有」，把快捷、状态、通知都塞在一起。把壳拆成 **窄顶栏常驻 + 底部 Dock**，顶栏就留给插件发挥——天气、时钟、窗口组、成语、正在播放……都有地方挂，也不用再挤一块粗任务栏。

最开始想做的是 **灵动岛**——顶栏中间那颗可展开、可通知、可拖放的胶囊。但只有一颗黑岛常挂在屏幕上，闲下来也会抢眼，体验并不舒服。于是做成了完整顶栏，并加上 **自动沉浸**：闲置后岛壳透底、跟环境反色，需要时再醒过来。岛仍然是主角，顶栏是让它能安静待着的壳。

顶栏上两块最值得自己玩的是：

- **快捷区**（左侧）— 插件条；可按 **当前前台进程 / 应用** 决定显示哪几个快捷插件（例如只在某 IDE 前台时出现对应工具条），也可以设成任意前台都显示  
- **灵动岛**（中间）— 天气、正在播放、消息横幅、中转站摘要……插件也可以直接往岛上发通知

右侧是系统托盘与 WLAN / 输入法 / 控制中心等 chrome；底下是 Dock。

| 表面 | 做什么 |
|------|--------|
| **顶栏** | 窄条常驻：快捷区 · 灵动岛 · 托盘与系统指示器；可沉浸，可代替任务栏占位 |
| **Dock** | 底部应用坞，可代替任务栏；显示模式、磁化放大、图标编辑 |
| **系统面板** | 控制中心、WLAN / 以太网、输入法菜单 |
| **插件** | 静态包（`.whpx`）+ CapGate（`hub.*`）；设置里本地安装 / 导入示例 |

技术上故意把插件面做薄：**一段简单的 HTML / JS，声明 capability，就能做出想玩的桌面小功能**——也方便用 AI 直接写插件，再推到快捷区或灵动岛上。这是灵动桌面最想坚持的一点。

窗口最大化时停在工作区边缘（AppBar 占位）；游戏与浏览器 F11 / 网页全屏时壳层会按策略隐藏。

## 界面一览

顶栏三段 + 底部 Dock：左侧快捷区（可按前台应用切换插件）、中间灵动岛、右侧托盘与系统指示器。

### 总览

<p align="center">
  <img src="docs/media/overview.png" alt="顶栏与 Dock 总览" width="900" />
</p>

| 区域 | 展示内容 |
|------|----------|
| 左侧快捷区 | 可按前台应用切换显示的插件条（世界时钟、窗口组等） |
| 中间灵动岛 | 天气摘要、媒体歌词、通知横幅等情景占位 |
| 右侧托盘 | 常显图标、WLAN / 电池 / 音量、输入法「中」、系统时钟 |
| 底部 Dock | 应用图标坞（分组、磁化放大、垃圾桶等） |

### 灵动岛

| 天气面板 | 媒体摘要 |
|:--------:|:--------:|
| <img src="docs/media/weather-panel.png" alt="天气面板" width="420" /> | <img src="docs/media/island-media.png" alt="媒体歌词" width="420" /> |
| 点击天气摘要展开详情（温度、湿度、风力） | 播放中时岛栏显示歌词 / 曲目摘要 |

| 消息通知 | 媒体 + 通知共存 |
|:--------:|:--------------:|
| <img src="docs/media/island-message.png" alt="消息通知" width="420" /> | <img src="docs/media/island-notify.png" alt="媒体与通知" width="420" /> |
| 通知横幅：「收到一条消息」等 Attention 槽位 | 媒体胶囊与绿色消息气泡可同时出现 |

<p align="center">
  <img src="docs/media/now-playing.png" alt="正在播放弹窗" width="720" />
  <br />
  <sub>正在播放：封面、进度与播放控制（插件弹窗）</sub>
</p>

### 右侧系统面板

| 控制中心 | WLAN |
|:--------:|:----:|
| <img src="docs/media/control-center.png" alt="控制中心" width="420" /> | <img src="docs/media/wlan.png" alt="WLAN 菜单" width="420" /> |
| Wi‑Fi / 蓝牙 / 热点、专注助手、亮度与音量、媒体快捷控制 | 开关、已连网络详情、扫描列表与偏好设置入口 |

### Dock 与设置

<p align="center">
  <img src="docs/media/settings-plugins.png" alt="设置与插件市场" width="720" />
  <br />
  <sub>设置窗：全局 / 主题 / Dock / 快捷区 / 托盘 / 插件市场（如世界时钟双时区）</sub>
</p>

Dock 常驻底部（可配置显示模式），图标按使用习惯分组；与顶栏 AppBar 一起占位工作区，普通最大化窗口不会盖住壳层。

## 功能概览

| 能力 | 说明 |
|------|------|
| 布局 | 认同 macOS「窄顶栏 + Dock」；顶栏常驻给插件留位，Dock 管启动与切换 |
| 灵动岛 | 最初的核心：折叠摘要、下拉面板、拖放中转、通知槽位；闲置可「自动沉浸」透底 |
| 快捷区 | 可按前台进程 / 应用显示对应快捷插件，也可设为全局常显 |
| Dock | 多种显示模式、热键呼出、图标编辑与悬停预览 |
| 控制中心 | Wi‑Fi / 蓝牙 / 热点、亮度与音量、媒体快捷入口 |
| 网络 | WLAN 扫描与连接；**有线优先**（插网线显示以太网） |
| 输入法 | 语言 / IME 芯片与切换菜单 |
| 托盘 | 原生钩子、常显钉选、闪烁可上岛通知 |
| 主题 | 玻璃材质、最大化窗口吸色（ambient） |
| 插件 | 快捷区 / 岛面板 / 托管弹窗；本地安装 `.whpx` 或导入示例；可用 `hub.notify` 等往灵动岛发消息 |

内置 / 可导入示例：天气、世界时钟、正在播放、中转站、文件搜索、窗口组、应用库、系统监控、成语、镜子、Excalidraw 等（见 `docs/plugins/examples/`）。想自己玩：读 [插件规范](docs/plugins/README.md)，用 AI 写一页 HTML 也能挂上顶栏。

## 技术栈

- **前端**：Tauri 2 · Vite 7 · React 19 · TypeScript
- **宿主**：Rust · SQLite · Win32（AppBar、Dock、托盘 hook、WLAN、IME、捕获、玻璃材质）
- **分发**：NSIS 安装包

## 环境要求

- **Windows 11**（当前主要适配与验收环境）
- Windows 10：理论上可运行，**尚未系统测试**
- [Rust](https://rustup.rs/)（MSVC toolchain）
- [Node.js](https://nodejs.org/) + npm
- WebView2（Win11 通常已预装）

## 快速开始

```bash
npm install
npm run tauri -- dev
```

| 命令 | 作用 |
|------|------|
| `npm run tauri -- dev` | 开发调试 |
| `npm run tauri:build:fast` | 快速 release（不打安装包） |
| `npm run tauri:build:bundle` | 打 NSIS 安装包 |
| `npm run check` | 架构边界 + 行为测试 + 前端构建 |
| `npm run plugins:check -- <目录名>` | 检查单个插件包 |
| `npm run clean:target` | 清理 Rust `target` |

产物：

- 可执行文件：`src-tauri/target/release/window-hub.exe`
- 安装包：`src-tauri/target/release/bundle/nsis/Window Hub_*_x64-setup.exe`

## 插件开发

| 文档 | 内容 |
|------|------|
| [docs/plugins/README.md](docs/plugins/README.md) | 规范总览 |
| [docs/plugins/development.md](docs/plugins/development.md) | 插件任务边界与工作流 |
| [docs/plugins/host-api.md](docs/plugins/host-api.md) | CapGate 与 `hub.*` |
| [docs/plugins/sdk.md](docs/plugins/sdk.md) | 分表面 SDK |
| [docs/plugins/plugin.schema.json](docs/plugins/plugin.schema.json) | `plugin.json` Schema |
| `docs/plugins/examples/` | 官方示例源 |

原则：**插件 = 静态包**；只使用宿主已声明的 capability；能力不足写在插件文档里，不要为做插件去改宿主。宿主把 `hub.*` 暴露清楚，就是为了让「简单 HTML + AI」也能做出能上岛、上快捷区的小功能。

## 项目结构

AI / 贡献者先读 [AGENTS.md](AGENTS.md)：[插件任务](docs/plugins/development.md)只改指定插件目录；[宿主任务](docs/development/README.md)按模块地图改系统。

```
window-hub/
├── src/                    # React 前端（岛、Dock、托盘、设置、弹窗）
├── src-tauri/              # Rust / Win32 宿主
│   └── resources/plugins/  # 内置插件运行包
├── plugins/                # 需编译的插件作者源（如 file-search）
├── scripts/                # build / checks / maintenance
├── tests/                  # 自动化验证
├── docs/development/       # 宿主开发入口与模块地图
├── docs/plugins/           # 插件契约与示例
├── docs/media/             # README 界面截图
├── docs/archive/           # 历史资料（非当前指令）
├── workspace/              # 本地日志与临时截图（不提交）
└── .cursor/skills/         # 岛 / Dock / 插件等开发细则
```

## 贡献

文件归属见 [工作区约定](docs/development/workspace.md)。改动前请对照相关 `.cursor/skills/`，保持岛栏几何、强调色与插件表面一致。

反馈请尽量附带：Windows 版本、复现步骤、日志或录屏。

## 许可

根目录 **尚未添加 `LICENSE`**。使用、分发或二次开发前请先与维护者确认；第三方组件以其各自许可证为准。

---

## English

**Window Hub** (also **灵动桌面**) is a Windows desktop shell inspired by macOS’s efficient **narrow menu bar + Dock**: a thin always-on top bar gives plugins room to live, while the Dock handles launch and switching. Both can replace the Windows taskbar.

The product started as a Dynamic Island; the full top bar and auto-immerse exist so the island can fade into the chrome instead of sitting as a permanent black pill. The **shortcuts** strip can show different plugins depending on the **foreground app/process**.

Screenshots: see **界面一览** above / [`docs/media/`](docs/media/).

### Highlights

- Layout — macOS-style narrow top bar + Dock; top bar reserved for plugins
- Shortcuts — per-foreground-app plugin visibility (or always-on)
- Island — panels, staging, notifications; plugins can push attention via `hub.notify`; optional auto-immerse
- Dock — taskbar alternative; display modes, magnification, icon editor
- Plugins — thin HTML/JS packages (`.whpx` / CapGate); easy for humans or AI to author

### Develop

```bash
npm install
npm run tauri -- dev
npm run check
npm run tauri:build:bundle   # NSIS
```

Requires **Windows 11** (primary). Windows 10 is untested. Also needs Rust (MSVC), Node.js, and WebView2.

### License

No root `LICENSE` yet — confirm with maintainers before redistribution.
