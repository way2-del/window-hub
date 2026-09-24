# Window Hub 插件规范

本目录与 [`.cursor/skills`](../../.cursor/skills) 共同构成插件平台契约。

| 文件 | 内容 |
|------|------|
| [development.md](development.md) | Agent 插件工作流、只改插件的边界与任务范围检查 |
| [plugin.schema.json](plugin.schema.json) | `plugin.json` JSON Schema |
| [host-api.md](host-api.md) | CapGate 与 `hub.*` |
| [sdk.md](sdk.md) | 分表面 `window.hub` |
| [local-storage.md](local-storage.md) | `plugin_kv` 规范 |
| [companion-scripts.md](companion-scripts.md) | Companion 边界与自启 |
| [marketplace.md](marketplace.md) | 本地安装 → 市场草图 |
| [examples/window-groups](examples/window-groups/) | 快捷区 + 弹窗示例 |
| [examples/app-library](examples/app-library/) | 快捷区悬浮入口 + 可自建应用库 |
| [examples/transfer-station](examples/transfer-station/) | 拖放暂存 + 岛面板示例 |
| [examples/weather](examples/weather/) | 岛栏常驻天气摘要 + 下拉详情 |
| [examples/idiom](examples/idiom/) | 快捷区成语 chip + 悬停释义 / 点击切换 |

**总 skill（AI 跑通流程）：** `.cursor/skills/window-hub-plugin/SKILL.md`

**原则：** 插件 = 静态包；只使用宿主已公开 API（声明 capability）。能力缺口记录在插件文档，不在插件任务中修改宿主；Companion 仅使用已开放且任务授权的协议。
**命名：** 顶栏左区 = **快捷区**；Dock 仅指后续底部坞。
