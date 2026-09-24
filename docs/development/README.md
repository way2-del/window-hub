# 宿主开发入口

这里面向开发 Window Hub 系统本身的 agent。插件任务转到 [插件工作流](../plugins/development.md)，不需要修改宿主来完成插件实现。

1. 读根目录 [AGENTS.md](../../AGENTS.md)，确认任务属于插件还是宿主。
2. 读 [模块地图](architecture.md) 定位领域，再读目标目录的 AGENTS.md / skill。
3. 按 [工作区约定](workspace.md) 保存源码、测试、开发记录与临时产物。
4. 小步改动：提取纯策略 → 保持调用行为 → 测试 → 再迁移 UI/生命周期；避免同时改动持久化键、事件名、窗口 label。
5. `npm run check` 验证前端结构与策略；Rust 变更另外 cargo check，原生窗口/托盘/焦点改动需要 Windows 实测。

当前可独立测试的模块：`src/app/windowRouting.ts`、`src/features/chrome/tokens.ts`、`src/features/island/{motion,geometry,pullContent}.ts`。这些模块不能反向拉入窗口组件、全局偏好或 Tauri。

已有全局 CSS 和原生状态机尚未整体迁移；继续按模块地图中的顺序处理，不以“清理结构”为由重写所有生命周期。
