# 内置插件源码

这里是**内置插件运行代码**的维护源；`docs/plugins/examples/<id>/` 是对应发布示例。只改当前任务的插件。

**宿主只读**：即使本目录在 `src-tauri/` 下，也不代表允许修改 Rust、bridge、全局 CSS 或插件安装器。插件开发只消费公开 `hub.*` API；缺能力写在插件自身 README 中，等待独立宿主任务。完整流程见 `docs/plugins/development.md`。

1. 读 `.cursor/skills/window-hub-plugin/SKILL.md`，再按表面选择快捷区/面板/弹窗 skill；修改前执行 `npm run plugins:scope -- begin <id>`。
2. 先运行 `npm run plugins:check -- <id>` 看当前差异。历史差异可能有文档用途，先检查再决定；不要直接批量覆盖。
3. 改这里的源文件；同步使用 `npm run plugins:sync -- <id>`。工具只同步一个插件，不删除示例独有文件；独有文件需人工判定用途。
4. 再执行 check、审查 diff、验证渲染与 hub 调用，最后 `npm run plugins:scope -- end <id>`。`npm run build` 不会检查这些独立 JS 的全部运行行为。

生成例外：`file-search/board.js` 的源是 `plugins/file-search/board.tsx`，用 `npm run build:file-search-board` 生成两份。`vendor/` 第三方内容不要手改。

更新源码不等于更新用户已安装副本。正式内置版本发布须核对 `src-tauri/src/plugin_install.rs` 的版本判断；开发目录通过现有 dev 安装方式加载。不要直接覆盖用户数据目录。
