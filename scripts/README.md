# 脚本索引

| 目录 | 用途 | 副作用 |
| --- | --- | --- |
| `checks/` | 结构检查、插件副本检查、任务范围检查 | `plugin-examples sync` 写一个示例目录；scope 写本地基线 |
| `build/` | 构建、启动、打安装包 | 可能停止并重启 Window Hub |
| `dev/` | 资源监控 | 读取进程/DB 锁状态，日志写 workspace |
| `maintenance/` | 清编译缓存、移除旧系统集成 | 会停止进程或修改系统状态，按明确任务使用 |

常用入口保持在 package.json：`npm run check`、`plugins:check`、`plugins:sync`、`plugins:scope`、`build:file-search-board`、`clean:target`。

自动化测试在 `tests/`。插件 TSX 源在 `plugins/<id>/`，不是构建工具；不要把业务代码放回 scripts。
