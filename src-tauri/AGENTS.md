# Rust / Win32 边界

插件开发任务禁止修改 `src-tauri/src/`、Cargo 配置、权限和安装逻辑；仅 `resources/plugins/<id>/` 属于该插件的实现范围。下列规则用于用户明确提出的宿主开发任务。

- `src/lib.rs` 负责启动、状态注册、命令注册和窗口生命周期装配。
- `src/commands.rs` 是现有 IPC 门面；新增独立逻辑放对应领域模块，门面只转发/校验。提取时保留命令名、参数名和返回 DTO，先核对 `generate_handler!` 与前端调用。
- `src/win32/` 处理 Windows API；`src/dock/`、`src/ecs/`、`src/db/`、`src/everything/` 各自负责领域。不要因插件 UI 改动修改原生窗口逻辑。
- `src/plugin_hub.rs` / `src/plugin_install.rs` 是权限与安装边界；新增 hub API 同时核对前端 bridge、capability 与 `docs/plugins/host-api.md`，不能只增加前端方法。
- 保留已有线程约束：同步 IPC / 后台线程中调用 WebviewWindow 的部分窗口操作可能死锁，遵守附近注释和现有 Win32 路径。不能按“统一封装”直接替换。
- 数据库键、事件名、窗口 label、插件版本升级逻辑均有跨语言消费者。改前搜索 Rust 和 TypeScript；需要迁移时明确兼容旧数据。

验证：`cargo check --manifest-path src-tauri/Cargo.toml`，并对变更领域做 Windows 实机验证。不要为验证停止用户正在运行的应用，除非当前任务已授权。
