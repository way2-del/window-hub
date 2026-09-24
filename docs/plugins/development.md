# Agent 插件开发工作流

**插件是宿主公开 API 的消费者。插件开发不修改 Window Hub 整体系统。**

## 先确定范围

只编辑本次插件的三个位置（使用目录名，例如 `world-clock`，不是 manifest 的完整 id）：

- `src-tauri/resources/plugins/<id>/`：运行包，简单插件直接在这里实现。
- `docs/plugins/examples/<id>/`：对应发布示例；通常由同步工具更新。
- `plugins/<id>/`：可选编译源，如 `file-search/board.tsx`；不要建立第二份 manifest。

宿主前端 `src/`、Rust `src-tauri/src/`、全局样式、hub bridge、Cargo/Tauri 配置、数据库和其他插件都只读。**不要因为现有 API 不满足插件需求，就替插件扩展或改写宿主。**

## 开发步骤

1. `npm run plugins:scope -- begin <id>`：保存任务开始时的工作文件指纹，兼容仓库已有未提交改动。不要覆盖活动基线。
2. 查 [host-api](host-api.md)、[SDK 表面矩阵](sdk.md)、[manifest schema](plugin.schema.json)，再读 `.cursor/skills/window-hub-plugin/SKILL.md` 和所用表面的 skill。
3. 用已有 `window.hub.*` 和声明式 settings/slots 实现。不得直接访问宿主内部模块、绕过 capability 或复制宿主逻辑到插件。文档与运行能力不一致时记录事实，不自行扩大权限。
4. 如果需要的能力未公开，在插件 README 写明需求、已查 API、受影响功能、可用降级。继续完成不依赖缺失能力的部分；由用户另行提出宿主 API 开发任务。Companion 只在现有公开协议支持且任务授权范围内使用，不作为绕过边界的方法。
5. 检查修改文件的 JS 语法、设置存取和相应表面交互；轮询/定时任务跨两次刷新。TSX 先运行已有对应构建命令。
6. `npm run plugins:check -- <id>` 查看副本差异；核对后 `npm run plugins:sync -- <id>`，再 check。历史文档/第三方资源差异需要说明，不批量覆盖其他插件。
7. `npm run plugins:scope -- end <id>`：通过范围检查后结束任务；说明验证结果、API 缺口和是否已加载到运行中的宿主。

范围检查只比较 Git 可见工作文件内容，不是操作系统沙箱，也不检查忽略目录里的运行行为。它能检测本次任务新增/修改/删除的越界文件；发现其他任务并发修改时先确认归属，禁止为了通过检查自动回滚他人文件或重建基线。

新插件进入内置市场、添加打包资源清单、变更全局依赖属于**宿主集成任务**，不是插件实现的隐含步骤。插件可先通过现有开发目录安装方式验证；不要为自动发现去改 `plugin_install.rs` 或市场目录。
