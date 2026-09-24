# 工作文件夹约定

## 两个 src 为什么保留

`src/` 是 Vite/TypeScript 前端编译单元；`src-tauri/src/` 是 Cargo/Rust 编译单元。这是现有 Tauri 工程的职责边界，不是重复源码。合并/改名会改变工具链和资源路径，却不会自动降低耦合，因此保留。

## 目录职责

```text
AGENTS.md                      agent 的任务分流与范围规则
src/                           宿主前端
  app/                         窗口启动策略与注入字段
  features/<domain>/           按领域拆出的逻辑
  plugins/                     宿主插件框架 / bridge（不是插件实现）
src-tauri/
  src/                         Rust / Win32 宿主
  resources/plugins/<id>/      内置插件运行包的维护源
  vendor/                     Rust/原生第三方依赖（不随意改）
plugins/<id>/                  需要编译的插件作者源文件，如 TSX
docs/
  development/                活跃宿主开发文档
  plugins/                    公开插件 API / 规范 / 对应示例
  archive/                    历史故障、恢复代码、旧插件包
scripts/
  build/                      构建/启动实现
  dev/                        可复用调试工具
  maintenance/                清缓存、卸载残留等显式维护操作
  checks/                     无业务副作用的结构/同步/范围检查
tests/                        自动化测试和最小辅助代码
.cursor/skills/               表面专项开发细则
workspace/                    本地任务产物（除入口说明外不提交）
  logs/                       原始日志
  screenshots/                视觉验证
  tmp/                        临时脚本、对比快照
  task-scopes/                插件任务开始时的文件指纹
  archive/<date>/             本地旧调试产物
```

## 放置与归档规则

- 新增长期架构/操作说明进 `docs/development/`；公开 API 契约进 `docs/plugins/`，插件专属说明留在插件目录。
- 已结束的排障记录移入 `docs/archive/<date>/`，标明历史状态；不要当成可重复执行的当前步骤。
- 一次性 md、日志、截图、调试脚本进 `workspace/<类别>/`。可复用调试脚本经过整理后再移入 `scripts/dev/`。
- 已恢复的源码副本在 `docs/archive/recovered-code/`；它们不参与编译，不是当前实现，不应被 agent 直接复制覆盖。
- 旧 `.whpx` / `.zip` 在 `docs/archive/plugin-packages/`。新临时打包产物进 `workspace/`；正式 NSIS 仍在 Cargo target 目录。
- `dist/`、`node_modules/`、`src-tauri/target/` 仍使用工具默认位置并忽略提交。不因整理目录删除缓存或用户数据。
- 根目录的 build/run 脚本只保留兼容转发，实际逻辑在 `scripts/build/`。BAT 和 PowerShell 共用一份实现，避免修一份漏一份。
- 路径移动后同步 npm scripts、文档链接、测试 fixture 和 `$PSScriptRoot` 相对路径。不能只移动文件不验证入口。

## 构建与维护不是只读验证

`build-and-run`、安装包脚本与 `clean:target` 可能停止运行中的 Window Hub；卸载脚本会修改系统集成。agent 做普通插件开发时不要调用这些操作。前端使用 `npm run check`；调试宿主或安装插件按用户当前授权进行。

`scripts/dev/monitor-resources.ps1` 默认写入 `workspace/logs/` 的时间戳文件。原生程序自带的 `%TEMP%` 日志仍按现有实现生成，需要保留证据时复制到 workspace，避免为日志归档修改运行行为。
