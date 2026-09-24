# 本地任务产物

除本说明与 AGENTS.md 外，本目录不提交。

- `logs/`：原始日志；`screenshots/`：验证截图。
- `tmp/`：一次性脚本、对比快照、任务 md。
- `task-scopes/`：插件任务基线，由 `npm run plugins:scope` 管理。
- `archive/<date>/`：归档的旧本地调试文件。

可复用工具/长期结论应整理进 `scripts/dev/` 或 `docs/development/`。不要在这里维护唯一的产品源码；不要自动运行归档脚本。
