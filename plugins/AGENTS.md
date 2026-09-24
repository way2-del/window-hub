# 插件作者源文件（不属于宿主）

简单插件直接维护 `src-tauri/resources/plugins/<id>/`。只有需要 TSX/编译的插件，才在本目录保存对应 `<id>/` 的作者源文件；不要复制整个运行包或建立第二套 manifest。

- `file-search/board.tsx` → `npm run build:file-search-board` → 内置与示例的 `board.js`。
- 本目录代码不能 import 宿主 `src/` 或 Rust 内部实现。使用插件已有公开 API 与自身依赖。
- 插件任务只改当前 `<id>/` 及其运行/示例目录，修改前后执行 `plugins:scope`。
- 构建输出保留在运行包中便于分发；临时 bundle、截图和调试脚本进 `workspace/`。

完整入口：[插件开发](../docs/plugins/development.md)。
