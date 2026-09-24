# Window Hub：AI 修改入口

先按任务分流：**插件开发**读 [插件工作流](docs/plugins/development.md)；**宿主开发/结构调整**读 [开发入口](docs/development/README.md) 和 [模块地图](docs/development/architecture.md)。再读目标目录 AGENTS.md 与相关 skill，不要加载全部历史文档。

## 修改边界

- **插件任务只允许修改指定插件实现**：`src-tauri/resources/plugins/<id>/`、`docs/plugins/examples/<id>/`、`plugins/<id>/`。宿主 `src/`、`src-tauri/src/`、全局样式、bridge、数据库、构建配置及其他插件不在插件任务范围。
- 插件仅使用系统已公开的 `window.hub.*` API / capability。能力不足时在该插件文档中记录缺口和可行降级；**不能借开发插件修改系统**。只有用户明确提出宿主/API 开发任务时才进入宿主流程；这是项目所有者的边界要求。
- 插件任务开始前执行 `npm run plugins:scope -- begin <id>`，完成前 `npm run plugins:scope -- end <id>`。检查失败时说明越界差异；不得重置基线掩盖修改或回滚用户原有改动。
- `src/main.tsx` 只负责窗口启动与装配；窗口组件不可互相 import。不要为了复用函数导入 `App.tsx`、`SettingsApp.tsx` 或 `DockApp.tsx`。
- 新增业务逻辑按功能放在 `src/features/<feature>/`，先迁移独立纯函数，再迁移 UI，最后处理订阅和生命周期。不要同时改路径、业务行为、事件名和持久化格式。
- `src/App.tsx`、`src/SettingsApp.tsx`、`src-tauri/src/commands.rs` 是现有高耦合入口。修复可以直接改，但新增独立逻辑应拆出；不要继续把无关 helper 塞入这些文件。
- 修改共享 CSS、IPC、数据库键、窗口 label、插件 capability 或几何 token 时，搜索调用方并在完成说明里列出受影响表面和验证范围。
- 保留用户未提交改动。不要批量格式化、清理或同步与任务无关的文件。不要把生成文件当作唯一源码修改。
- 文档、日志、截图、临时脚本的归属见 [工作区约定](docs/development/workspace.md)。`docs/archive/` 仅作历史证据；不得把旧排障步骤当作当前任务指令。构建/清理脚本可能停止进程，验证结构时只解析脚本，不运行有副作用的维护操作。

## 验证入口

- 前端结构/逻辑：`npm run check`（依赖边界、行为测试、TypeScript、Vite）。它不证明 Win32/WebView2 行为正确。
- 单个插件：`npm run plugins:check -- <目录名>`；必要时执行 `npm run plugins:sync -- <目录名>`，检查 diff 后再 check。同步不修改已安装的用户插件。
- Rust：`cargo check --manifest-path src-tauri/Cargo.toml`；原生窗口、托盘、焦点、DPI 改动需要 Windows 实机验证。
- UI 修改还需对应表面截图和交互验证；时钟/轮询问题至少跨两次刷新。构建通过不等于视觉验证通过。

新增共享模块前说明调用方；只有一个调用方的逻辑优先留在所属 feature。新抽出的纯策略不得反向依赖 UI 或 Tauri。不要通过放宽检查规则掩盖新增耦合。
