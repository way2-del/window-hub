# 第三方本地脚本（Companion Script）与 Window Hub 的边界

本文划清：**`.whpx` 前端插件**、**宿主 CapGate 能力**、**用户自备本地脚本服务**三者的关系。  
Skill 开发与 Agent 生成插件时必须遵守。

## 关系总览

```text
┌─────────────────────────────────────────────────────────┐
│  Window Hub 宿主进程（Tauri / Rust）                      │
│  · 状态菜单插槽渲染                                        │
│  · CapGate：hub.windows / storage / notify / …           │
│  · 可选：在「插件设置」里登记脚本路径、开机自启、关联启动     │
│  · 不把第三方脚本 LoadLibrary 进主进程                      │
└───────────────┬──────────────────────────▲──────────────┘
                │ window.hub.*             │ HTTP / 本地 socket
                │（宿主已提供的系统能力）      │（脚本自己暴露的 API）
                ▼                          │
┌───────────────────────┐    ┌─────────────┴────────────────┐
│  .whpx 前端插件         │───▶│  用户本地 Companion 脚本       │
│  HTML/CSS/JS            │    │  Python / Rust / Node / …    │
│  声明 slots + capabilities│   │  独立进程；可开机自启          │
└───────────────────────┘    └──────────────────────────────┘
```

| 角色 | 属于谁 | 职责 |
|------|--------|------|
| **Window Hub** | 本仓库宿主 | 插槽 UI、CapGate、共享系统能力（如 WindowsService）；**可登记**脚本自启，**不执行插件业务逻辑于主进程** |
| **`.whpx` 插件** | 第三方 / 官方静态包 | 只含 Web UI + `plugin.json`；经 `window.hub` 调宿主；需要额外能力时再调本地脚本 API |
| **Companion 脚本** | **用户本机、插件作者自备** | 实现宿主未提供的能力；自管生命周期、端口、数据；与 Hub **进程隔离** |

**一句话：** Hub 提供「官方插座」；超出插座的，用用户选的语言生成本地脚本并封装成接口，由用户在插件设置里配置自启——脚本是邻居进程，不是 Hub 的 DLL。

## 何时用 CapGate，何时用本地脚本

1. **宿主已有 capability**（见 [host-api.md](./host-api.md)）→ **必须**声明后走 `window.hub.*`。  
   例：枚举窗口 → `windows.read` + `hub.windows.list`，禁止脚本再搞一套 `EnumWindows`。
2. **宿主没有、也不打算进基座的能力** → **不要**往 `.whpx` 里塞未审核原生二进制。  
   改为：用用户指定语言生成 **Companion 脚本**，封装 HTTP/JSON（或命名管道）接口，前端插件 `fetch`/`WebSocket` 调用。
3. **希望成为全平台官方能力** → 仍走「提案 → 进基座 → 新 capability」，而不是永久依赖私有脚本。

## 插槽（slots）与 capability（必须声明）

前端插件 `plugin.json` **只**声明 Hub 插槽与 CapGate；**不要**把 companion 脚本路径写进市场包强制绑定（路径因机器而异）。

### 常用 slots

| Slot | 含义 |
|------|------|
| `shortcuts` | 状态菜单快捷区 |
| `island.notify` | 灵动岛通知 |
| `island.panel` | 岛下拉面板 |
| （经 `entry.popup` + capability `popup`） | 托管弹窗 |

### 常用 capabilities

见 [host-api.md](./host-api.md) 表。缺失声明 → 调用被拒。

脚本侧能力（读任意文件、跑命令、自建 HTTP）**不在** CapGate 内，由用户安装脚本时自行承担风险；Hub 仅做「是否登记自启」的开关，不背书脚本内容。

## Companion 脚本约定（Agent / 作者生成时）

当用户需要系统能力之外的逻辑时，Agent **应直接生成**用户选择的语言实现，并满足：

1. **独立进程**：Python / Rust(exe) / Node 等，**禁止**要求 Hub `LoadLibrary` 插件 DLL。  
2. **封装成接口**：默认本机回环，例如 `http://127.0.0.1:<port>/...`，JSON 请求/响应；端口可配置，避免写死冲突。  
3. **前端调用**：`.whpx` 内 JS 使用 `fetch`（若走 Hub `network` capability，须声明并白名单 `127.0.0.1`；或文档约定仅开发态直连）。  
4. **生命周期（用户可配）**  
   - **开机自启**：写入用户启动项 / 任务计划（由 Hub「插件设置」或脚本安装器完成，**脚本仍属用户进程**）。  
   - **关联启动**：随 Window Hub 启动或随某插件启用而拉起；Hub 退出时可选择不杀脚本（默认不杀，避免误伤）。  
5. **性能**：空闲可退出；多插件禁止每人常驻一份同质系统枚举；能复用 `hub.windows.*` 的绝不在脚本里重复。

### 推荐目录（本机，不进 `.whpx` 运行时强制路径）

```text
%APPDATA%/window-hub/companions/{pluginId}/
  ├── run.ps1 / main.py / companion.exe
  ├── config.json          # port、自启开关等
  └── README.md            # 如何启动、API 列表
```

`.whpx` 内可附带 `companions/` **源码模板**供用户复制安装；**运行实例**在 AppData companions 下，与静态插件包分离。

## Window Hub「插件设置」职责（产品边界）

设置 → **插件市场** → **脚本启动器**（已落地）可配置：

| 配置项 | 含义 | Hub 做什么 | Hub 不做什么 |
|--------|------|------------|--------------|
| 脚本路径 | 可执行文件或入口脚本 | 登记路径、校验存在、浏览选择 | 不解析/沙箱执行业务 |
| 运行环境 | python / node / powershell / cmd / exe / custom | 按环境拼启动命令 | 不捆绑解释器 |
| 关联插件 | 可选 plugin id | 该插件**启用**时关联拉起 | 不把脚本并入 `.whpx` |
| 随 Hub 启动 | 启动 Window Hub 时拉起 | `CreateProcess` 独立进程 | 不注入 DLL |
| 开机自启 | 登录时启动 | 写入用户 Startup 下 `.cmd` | 不提升为系统服务 |

失败时前端应提示「companion 未运行」，而不是让 Hub 崩溃。  
配置落盘：`%APPDATA%/window-hub/companions/launchers.json`。

## 禁止事项

- 市场 `.whpx` **附带未审核** `.dll` / 要求注入宿主  
- 用 companion **替代** 已有 `hub.windows.*` 等高频系统能力  
- 在文档/Skill 中暗示「第三方脚本 = Window Hub 官方子系统」  
- 无用户确认即静默写入开机自启  

## 与窗口组示例的对照

官方窗口组：**只需** `shortcuts` + `storage` + `popup` + `windows.*`，**不需要** companion。  
仅当例如「连接公司私有硬件 / 私有协议 / 本地 ML」等 Hub 不会内置的能力时，才生成 companion。
