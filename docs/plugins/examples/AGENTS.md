# 插件示例

与 `src-tauri/resources/plugins/<id>/` 同名的插件，运行代码以资源目录为维护源。先在那里修改，再用 `npm run plugins:sync -- <id>` 同步；不要在两处分别手改同一段逻辑。

此目录也有仅用于教学的插件，它们不自动归入内置插件。示例独有 README/资源保留并按用途审查；同步工具不会自动删除。已有历史差异见 `npm run plugins:check -- --all`，不要在无关任务中全部覆盖。

历史压缩包已归档到 `docs/archive/plugin-packages/`，不作为最新源码。插件任务遵循 `docs/plugins/development.md` 的范围检查；不要为适配示例修改宿主 API。
