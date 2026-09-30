# 顶栏窗口契约

顶栏是常驻 Shell AppBar。三指下滑、显示桌面或全部最小化不应使它参与普通窗口的最小化动画。

## 所有权

- `appbar.rs` 将实际 `main` HWND 注册为 AppBar，不再创建透明占位窗口。工作区仍只保留 28 逻辑像素；`ABM_SETPOS` 后不缩放实际窗口，展开灵动岛不会增加占位。
- `appbar_window.rs` 由 setup 在 main 的 UI 线程安装 subclass：在默认窗口过程之前拒绝 `SC_MINIMIZE`，防止后续样式刷新重新加入 `WS_MINIMIZEBOX`，处理 Shell 通知和 Explorer 重建。
- AppBar 回调的通知码读取 `wParam`。Shell 调用由已有 worker 串行执行；UI 回调仅排队，避免在通知中同步重入占位计算。
- main 创建时即 `alwaysOnTop: true`、`minimizable: false`。收起灵动岛仍保持置顶，避免等待桌面前台轮询后才提升层级。后台调用 SetWindowPos 成功并不总能取得置顶权限，因此创建时的样式不可省略。
- 全屏隐藏继续由现有 fullscreen watcher 管理；启动隐藏、退出与托盘菜单临时让位保持原有职责。设置窗、Dock 和插件窗口不安装此 subclass。

## 验证

常规原生保护测试（创建独立测试 HWND，不操作用户程序）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib appbar_window::tests::minimize_is_rejected_before_default_window_processing
```

交互式 Shell 回归会暂时最小化窗口/显示桌面，并恢复；只在可交互的 Windows 桌面显式执行：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib appbar_window::tests::shell_show_desktop_preserves_visible_appbar_without_watchdog -- --ignored --nocapture --test-threads=1
```

测试覆盖未保护窗口确实可最小化、受保护窗口拒绝同一命令、框架样式刷新、应用主动隐藏、实际 HWND 的 Shell 注册、展开前后占位不变，以及无 watchdog 时的全部最小化和两轮显示桌面。

2026-09-24：以上测试通过；release-fast 构建成功并替换运行版本。真实顶栏两轮显示桌面共 60 次采样均可见、非最小化、保持置顶且可命中；截图位于本地 `workspace/screenshots/appbar-show-desktop.png`。物理触摸板三指手势未直接自动化；全屏游戏、多显示器/DPI 切换及 Explorer 重启尚未做本轮实机回归。

多显示器布局（顶栏 / Dock 分屏、副屏卫星顶栏）见 [display-placement.md](display-placement.md)。
