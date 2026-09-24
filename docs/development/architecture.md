# Window Hub 模块地图与渐进拆分

审查日期：2026-09-24。目标是让 AI 从任务直接定位到小模块，并让越界依赖尽早失败。

## 当前结论

| 观察 | 风险 | 本轮处理 |
| --- | --- | --- |
| App.tsx 约 4,046 行；commands.rs 约 4,282 行 | 多个功能共用状态/入口，AI 修改需要过多上下文 | 从 App 提取 chrome 颜色策略与图片采样；约束后续新增逻辑位置 |
| SettingsApp.tsx 约 2,595 行；DockApp.tsx 约 1,955 行 | 功能、设置、订阅混在窗口组件里 | 标记为分阶段迁移入口，未批量搬移 |
| main 静态导入各窗口，窗口识别与渲染在一起 | 入口改动影响多个原生窗口 | 独立 windowRouting 与注入字段声明；测试所有别名和优先级 |
| main 全局加载 App.css / settings.css（约 2,595 / 3,245 行） | 跨窗口 selector 副作用 | 禁止新增无范围覆盖；现有 CSS 保持加载顺序，后续逐表面迁移 |
| 内置插件与 examples 双份代码 | AI 改错副本、漏同步、源码与安装副本混淆 | 明确运行代码维护源，提供单插件同步和只读漂移检查 |
| 仅有构建，没有结构检查入口 | 耦合扩大只能靠人工发现 | 增加循环依赖、窗口入口反向依赖和已提取模块边界检查 |
| 约 72 个原有 TS/TSX 模块无运行时循环依赖 | 当前问题主要是文件内聚合与共享副作用 | 保持零循环，而非先迁移所有路径 |

行数是调整前快照，不是强制门槛。纯搬文件不消除状态/事件/CSS 耦合。

## 按任务找代码

| 任务 | 首选入口 | 需要扩大范围时 |
| --- | --- | --- |
| 单个插件显示/设置 | `src-tauri/resources/plugins/<id>/` | 只读公开 API 文档；能力缺口转独立宿主任务 |
| 快捷区尺寸与生命周期 | `src/components/ShortcutsHost*`、`ShortcutsPluginStrip.tsx` | `src/plugins/shortcutsGeometry.ts`、`shortcutsHubBridge.ts` |
| 岛交互、动画、通知 | `src/features/island/`、`src/App.tsx`、`src/plugins/island*` | 岛相关 skill；原生几何入口 |
| 顶栏吸色与文字对比度 | `src/features/chrome/` | App 负责应用结果；`win32/ambient.rs` 提供背景输入 |
| 窗口识别 | `src/app/windowRouting.ts` | `src/main.tsx` 装配、Rust 窗口 label |
| 设置 | `src/SettingsApp.tsx`、对应 `*Prefs.ts` | SQLite / Tauri 事件契约 |
| Dock | `src/DockApp.tsx`、`src-tauri/src/dock/` | Dock 专用样式、Win32 定位 |
| 系统任务栏与顶栏占位 | `src-tauri/src/win32/status_menu.rs`、`appbar.rs`、`appbar_window.rs`、`dock_appbar.rs` | 实际 main HWND 注册 AppBar；最小化保护与验证见 [顶栏窗口契约](appbar-window.md) |
| 托盘 / 网络 / 输入法 | 对应 `*PopupApp.tsx` 与 components | `src-tauri/src/win32/` 对应模块 |
| 原生 IPC | `src-tauri/src/commands.rs` | `lib.rs` 注册，领域模块实现 |
| 插件安装与权限 | `src-tauri/src/plugin_install.rs`、`plugin_hub.rs` | manifest schema、前端 bridge、host-api 文档 |

## 已建立的依赖方向

```text
main（启动装配） → 窗口组件 → feature / 领域服务
main → app/windowRouting（纯策略）
App → features/chrome/sampleStripBands（浏览器适配）
App → features/chrome/tokens（纯策略）
App → features/island/{motion,geometry,pullContent}（参数注入的纯策略）
插件静态页面 → hub bridge → Rust command → 领域 / Win32
```

窗口组件不可被底层模块导入。纯策略通过参数接收数据，不拉取全局 store 或调用 IPC。需要共享的是 DTO/小型策略/服务接口，不是整个窗口组件。

`npm run check:architecture` 使用 TypeScript AST 扫描相对路径的运行时 import、re-export 和字面量动态 import；类型依赖单独排除。它不覆盖 Rust、CSS、字符串事件以及计算型动态加载，不能代替跨语言搜索。新增路径 alias 时需要同步扩展解析器。

## 验证与同步

```sh
npm run check
npm run plugins:check -- world-clock
npm run plugins:sync -- world-clock
npm run plugins:check -- --all
cargo check --manifest-path src-tauri/Cargo.toml
```

全量插件检查目前会报告历史差异，因此不接入通用 `check`。初次语义检查发现 14 项（含源码独有文件和示例独有 README；换行差异不计入）。部分可能是有意的文档/第三方裁剪，须按插件确认。不要为了让全量检查变绿覆盖所有文件。

源码位置、发布示例、用户安装副本是三件事。工具只同步仓库内一个插件，不触碰安装副本、不删除文件。`file-search/board.js` 是生成文件，源入口在 `plugins/file-search/board.tsx`。

## 后续拆分顺序

1. 岛：路径数学、运动函数、内容选择策略已提取并有输入输出测试；下一步按通知、搜索、拖放拆订阅 hook。动画逐帧共享状态最后处理，保留一个生命周期所有者。
2. 设置：按设置页拆组件，每页只依赖本领域偏好适配器；先保持现有数据库键与通知事件。
3. Rust commands：按 Dock / 网络 / 插件 / 旧窗口捕获拆领域实现，保留门面转发和注册名。逐组 cargo check 加原生验证，不同时改 IPC 协议。
4. CSS：先列 selector 消费者，再逐窗口加作用域/迁移局部 CSS；确认依赖后才能移除 main 的全局 import。
5. 窗口加载：样式副作用理清后再按窗口动态 import，避免一次拆包改变 CSS 顺序。

每一步应是可单独构建、验证、回退的行为保持改动。本轮没有重写原生窗口状态机、修改数据格式或承诺消除全部耦合。

## 第二轮工作区整理

- 保留两个编译单元 `src/` 和 `src-tauri/src/`；目录职责见 [工作区约定](workspace.md)。
- `App.tsx` 继续移出约 190 行运动/形状/选择策略；几何通过工厂参数获得面板阈值，内容选择通过回调获得启用判断，不依赖持久化偏好或注册表。
- `scripts/` 按 build/dev/maintenance/checks 分类，测试移至 `tests/`，文件搜索 TSX 作者源移至 `plugins/file-search/`。
- 根目录 build/run 入口保留兼容转发；BAT 不再维护第二份构建流程。
- 活跃开发文档归入 `docs/development/`，旧排障/恢复代码/旧插件包归入 `docs/archive/`，原始本地调试文件移至忽略提交的 `workspace/archive/`。
- 插件任务执行 `plugins:scope begin/check/end`，以任务开始时工作区为基线检测越界修改；不把原有未提交变更误报为本次修改。边界流程见 [插件开发](../plugins/development.md)。
