# Window Hub

**Windows 顶栏壳** — 用灵动岛、Dock 与系统托盘 chrome 重新组织日常窗口与快捷操作。

> Windows 10+ only · [Tauri 2](https://tauri.app/) + React + Rust / Win32  
> [English summary](#english)

## 这是什么

Window Hub 不是又一个启动器，而是一层贴合 Windows 的 **宿主壳**：

- **灵动岛** — 顶栏折叠岛、下拉面板、拖放中转、情景占位、通知横幅、可选沉浸式反色
- **Dock** — 底部坞（显示模式、磁化放大、图标编辑、启动器等）
- **状态栏 chrome** — 快捷区插件条、系统托盘聚合、WLAN / 以太网与输入法指示器
- **插件宿主** — 静态包（`.whpx`）+ CapGate（`hub.*`），示例与本地市场入口齐全

旧版「窗口马赛克停靠 / 离屏捕获」能力仍在代码库中，但当前产品主轴是 **岛 + Dock + chrome + 插件**。

## 界面一览

顶栏三段 + 底部 Dock：左侧快捷区（前台应用 / 世界时钟等插件）、中间灵动岛、右侧托盘与系统指示器。

### 总览

<p align="center">
  <img src="docs/media/overview.png" alt="顶栏与 Dock 总览" width="900" />
</p>

| 区域 | 展示内容 |
|------|----------|
| 左侧快捷区 | 前台应用名、世界时钟、窗口组等插件条 |
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

## 功能亮点

| 能力 | 说明 |
|------|------|
| 灵动岛 | 岛栏摘要、下拉面板、拖放暂存、通知槽位 |
| Dock | 多显示模式、悬停预览、热键与设置面板 |
| 网络指示器 | Wi‑Fi 扫描 / 连接；**有线优先**（插上网线显示以太网图标与详情） |
| 输入法 | 语言与 IME 状态芯片、快捷切换菜单 |
| 托盘 | 原生托盘钩子、常显钉选、闪烁上岛通知 |
| 插件 | 快捷区 / 岛面板 / 托管弹窗；官方示例可直接导入 |

## 技术栈

- **UI**：Tauri 2 · Vite · React 19 · TypeScript
- **后端**：Rust · `bevy_ecs` · SQLite
- **系统**：Win32（托盘 hook、AppBar / Dock、WLAN、IME、捕获、玻璃材质等）
- **分发**：NSIS 安装包

## 环境要求

- Windows 10 或更高
- [Rust](https://rustup.rs/)（MSVC toolchain）
- [Node.js](https://nodejs.org/) + npm / pnpm
- WebView2（Win10/11 通常已预装）

## 快速开始

```bash
npm install
npm run tauri -- dev
```

### 常用脚本

| 命令 | 作用 |
|------|------|
| `npm run tauri -- dev` | 开发调试 |
| `npm run tauri:build:fast` | 快速 release（不打安装包） |
| `npm run tauri:build:bundle` | 打 NSIS 安装包 |
| `npm run clean:target` | 清理 Rust `target` |

产物示例：

- 可执行文件：`src-tauri/target/release/window-hub.exe`
- 安装包：`src-tauri/target/release/bundle/nsis/Window Hub_*_x64-setup.exe`

## 插件开发

| 文档 | 内容 |
|------|------|
| [docs/plugins/README.md](docs/plugins/README.md) | 规范总览 |
| [docs/plugins/host-api.md](docs/plugins/host-api.md) | CapGate 与 `hub.*` |
| [docs/plugins/sdk.md](docs/plugins/sdk.md) | 分表面 SDK |
| [docs/plugins/plugin.schema.json](docs/plugins/plugin.schema.json) | `plugin.json` Schema |
| `docs/plugins/examples/` | 窗口组、中转站、天气、文件搜索等示例 |

原则：**插件 = 静态包**；系统能力由宿主声明（capability）；超出边界用 Companion 独立进程。

## 项目结构（简）

AI 开发先读 [AGENTS.md](AGENTS.md)。[插件任务](docs/plugins/development.md)只实现插件并使用公开 API；[宿主任务](docs/development/README.md)按模块拆分。结构检查与行为测试：`npm run check`；单插件检查：`npm run plugins:check -- <目录名>`。

```
window-hub/
├── src/                    # React 前端（岛、Dock、托盘、弹窗）
├── src-tauri/              # Rust / Win32 宿主
│   └── resources/plugins/
├── plugins/                # 需要编译的插件作者源（如 file-search TSX）
├── scripts/                # build / dev / maintenance / checks
├── tests/                  # 自动化验证
├── docs/development/       # 活跃开发规划与模块地图
├── docs/plugins/           # 插件契约与示例
├── docs/media/             # README 界面截图
├── docs/archive/           # 历史排障、恢复副本与旧包
├── workspace/              # 本地日志、截图、任务记录（不提交）
└── .cursor/skills/         # 开发约定（岛、插件、托盘等）
```

## 贡献

完整文件归属见 [工作区约定](docs/development/workspace.md)。`src/` 与 `src-tauri/src/` 分别是前端与 Rust 编译单元，保留分离。

欢迎 Issue / PR。改动前建议先阅读对应 `.cursor/skills/` 与 `docs/plugins/`，保持岛栏几何、强调色与插件表面一致。

反馈时请尽量附带：Windows 版本、复现步骤、日志或录屏。

## 许可

本仓库根目录 **尚未添加 `LICENSE` 文件**。使用、分发或二次开发前请先与维护者确认许可意向；引入的第三方组件以其各自许可证为准。

---

## English

**Window Hub** is a Windows-only desktop shell built around a Dynamic Island, Dock, and system-tray chrome.

Screenshots of the top bar and Dock live under [`docs/media/`](docs/media/) (see the Chinese **界面一览** section above).

### Highlights

- Dynamic Island — compact bar, pull-down panels, drop staging, notifications
- Dock — display modes, hover previews, hotkeys
- Chrome — shortcuts strip, tray aggregation, **WLAN + Ethernet** (wired preferred), IME chips
- Plugins — static `.whpx` packages via CapGate (`hub.*`); see `docs/plugins/`

### Develop

```bash
npm install
npm run tauri -- dev
npm run tauri:build:bundle   # NSIS installer
```

Requires Windows 10+, Rust (MSVC), Node.js, and WebView2.

### License

No root `LICENSE` yet — confirm with maintainers before redistribution.
