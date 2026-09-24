# 启动、重启与退出

`src-tauri/src/lifecycle.rs` 由宿主入口和退出命令调用，独立监督顶栏原生显示、React 挂载及 Dock 初始化。前端 `src/app/StartupSurface.tsx` 在提交后的两个动画帧通过 `startup_surface_ready` 回报；后端使用调用窗口的原生 label，忽略其他窗口。该回报表示 React 已挂载且获得绘制机会，不等同于持续运行健康检查或像素检测。

托盘采用渐进加载：启动不等待托盘 seed；沿用 `tray-icons` 的约 400ms 合并推送。顶栏先订阅再拉快照，并拒绝覆盖更新事件的过期快照。顶栏与收纳弹窗共用 `features/tray` 的补图队列：每个表面一次最多 6 项、批次间隔 100ms、同一时刻一个在途请求；缺图最多尝试 3 次，重试间隔 1500ms，新项优先于重试。卸载或图标移除后忽略旧补图结果。PNG 查询和列表查询在后台 IPC 执行，不占用主窗口线程。已有内容直接显示，不等待该队列清空。

- 启动超过 45 秒未就绪：后台线程显示原生错误对话框，列出缺失阶段和 `%TEMP%/window-hub-boot.log`。用户可选择退出或继续等待。Dock 创建错误、20 秒超时及任务中断即时提示，同一启动只提示一次。
- Tauri 创建窗口/执行 setup 失败：显示原生错误对话框后退出。
- 重启子进程携带 `--wait-for-restart`，最多等待 15 秒获取单实例锁。旧进程保留锁直到真正结束，避免两个实例同时操作 AppBar。
- 菜单退出/重启先设置停止标志，释放上下 AppBar、恢复系统任务栏，再请求 Tauri 退出。独立 6 秒期限处理退出消息循环卡死；仅结束当前进程，不按进程名杀其他实例或服务。
- AppBar 命令、顶栏 watchdog 和隐藏系统任务栏请求在停止期间停止重新占位。托盘统计始终读取原子快照，零值不触发图标表加锁。

## 已定位的 TaskbarCreated 自锁（2026-09-24）

失败样本主线程的 Wait Chain 指向 Explorer，同步调用栈为 `tao::window::set_skip_taskbar` → `ITaskbarList` / `SendMessageW` → 重入 Tao 窗口回调 → `parking_lot::RawMutex::lock_slow`。Tao 0.35.3 的 `S_U_TASKBAR_RESTART` 分支在持有 `window_state` 时调用 `set_skip_taskbar`，重入后会再次获取该锁。

本项目原先两处合成 TaskbarCreated 广播会触发此路径：宿主托盘补采集、vendor systray-util 初始化。两者现在共用 vendor 的 `refresh_taskbar_icons`：枚举顶层窗口，排除自身进程及无效 owner，再 PostMessage 给外部进程。真实 Explorer 重启消息和框架实现没有修改；此修复针对应用主动发送的合成消息。该 vendor 补丁直接修复已确认的启动死锁，升级库时必须保留或确认上游已解决。

启动阶段额外记录 IPC 命令名的进入/返回，不记录参数；用于区分命令阻塞和窗口回调阻塞。调用方：宿主 tray 和 vendor TraySpy。影响托盘刷新及启动可靠性，不改变插件 network 权限或配置。

## 已定位的 Dock 布局互等

另一失败样本在 READY 后卡死：主线程的 IPC `dock_relayout` → `place_dock_window` 等待 `dock_place_lock`；后台布局持锁后需要 UI 消息循环执行窗口操作。Dock 布局相关 IPC 使用 `#[tauri::command(async)]`，保留 Rust 同步函数和 IPC 参数/返回值，让 IPC wrapper 在线程池执行。拖放后台调用仍可直接调用原函数。调用方包括 Dock 初次刷新、设置中的放大预览、固定/取消固定/导入，以及状态菜单恢复隐藏项。

45 秒监督还使用缓存的 main HWND 检查 `IsHungAppWindow`，不向可能卡死的 Tauri UI 发起同步请求；因此即使已回报挂载、已经显示，消息循环无响应也会提示。

## 检查入口

自动化：`npm run check`；`cargo check --manifest-path src-tauri/Cargo.toml`；`cargo test --manifest-path src-tauri/Cargo.toml --lib startup_counts_do_not_wait_for_icon_registry`。

Windows 实测需覆盖：正常启动顶栏与启用的 Dock、菜单重启、菜单退出再打开、禁用 Dock 启动、WebView 创建失败弹窗、启动阶段停滞弹窗及从弹窗退出。原生行为不能仅凭编译通过确认。

影响表面：顶栏、Dock、状态菜单退出/重启、系统任务栏恢复、单实例启动，以及所有通过 main 装配的窗口（新增就绪回报仅接受 main/dock）。未更改数据库格式、已有窗口 label、几何 token 或插件 API。
