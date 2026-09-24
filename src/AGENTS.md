# 前端边界

根目录 AGENTS.md 与 [模块地图](../docs/development/architecture.md) 适用。

**本目录是宿主前端；插件实现任务只读本目录，不在此添加插件专属逻辑。**

- `app/windowRouting.ts`：纯窗口识别策略。优先级为注入标记 → 原生 label → URL → island。这里不读取 DOM、不调用 Tauri，输入从 main 注入。
- `app/windowGlobals.d.ts`：宿主注入字段声明；字段名是前后端契约。
- `features/chrome/tokens.ts`：纯颜色/对比度策略；`sampleStripBands.ts`：浏览器图片采样。两者不依赖岛状态、偏好存储或插件注册表。
- `features/island/`：形状几何、运动计算、内容优先级。几何阈值/启用判断由调用方注入；禁止导入持久化偏好或 registry。原生窗口操作和订阅仍由 App 管理。
- `plugins/`：manifest、registry、bridge 和插件表面契约。插件自身的日期/天气/任务业务留在插件资源目录。
- `components/` 与根目录 `*App.tsx` 是旧布局，按实际修改逐步迁移；不要单为整理目录移动全部文件。
- 新 feature 的内部文件使用直接 import。避免一个大 `index.ts` 重导出所有窗口、服务和组件。
- 现有 `App.css`、`settings.css` 由 main 全局加载。当前不能直接删除其 import；先清点 selector 消费者。新增局部样式用 feature 前缀/窗口根 class，不增加裸 `button`、`body`、`*` 覆盖。

相关 skill：岛布局 `window-hub-island-panel`，快捷区 `window-hub-shortcuts`，状态菜单 `window-hub-status-menu`，插件宿主 `window-hub-plugin`，插件弹窗 `window-hub-plugin-popup`，强调色 `window-hub-system-ui-accent`（均位于 `.cursor/skills/`）。只读任务相关项。
