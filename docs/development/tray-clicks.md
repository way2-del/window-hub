# 托盘点击：回调与原生兜底

## 适用入口

顶栏 `TrayCluster`、收纳面板 `TrayPopupApp` 共用 `trayInvoke.ts` →
`invoke_tray_icon`。岛通知和绑定托盘的插件也最终调用 `tray::invoke_icon_by_id`。
IPC 名称、参数、托盘偏好和收纳/常显持久化格式未改。

## 失败原因与处理

托盘注册表缓存只有图片、名称、GUID 等信息，不包含应用接收点击的窗口和回调。
`hwnd=0 callback=0` 的图标即使能显示，补发 `WM_LBUTTONUP` 或更改协议打包也无效。
原来的恢复路径广播 TaskbarCreated、安装观察钩子并等待重新注册；应用不重新注册时仍失败。

- 有存活窗口和回调：继续走原有消息转发，保留 PixPin 等已经工作的行为。
- 缺少回调：`tray_native.rs` 在后台线程定位 Explorer 原生托盘按钮。
  Win11 搜索 SystemTray 类的按钮，Win10 搜索 TrayNotifyWnd，避免点到任务栏应用按钮。
- 匹配使用完整名称、提示首行或完整进程名称，归一化空白；不做任意子串匹配。
  同一区域中有多个匹配时返回错误，不随机选一个。
- 左键优先使用 Invoke；右键优先使用 ShowContextMenu，不将右键退化成 Invoke。
  双击及不支持相应模式的按钮，将鼠标消息投递到匹配按钮位置的 Explorer 输入子窗口。
- Windows 收纳图标先展开原生收纳窗口，再查找按钮。Hub 的常显设置不改变 Windows 区域。
- Window Hub 隐藏任务栏时，兜底期间短暂显示系统任务栏。
  `status_menu::hold_taskbar_for_tray` 暂停隐藏巡检；成功后延迟释放，失败则立即释放，
  按当前用户任务栏设置恢复隐藏。不会改 IsPromoted、收纳注册表或持久化显示设置。
- 同时只允许一个原生兜底请求；UIA 使用有限的连接/事务超时，避免连续点击积压。
- 异步 IPC 等待后台线程结果，失败会 reject 并写点击日志；消息投递成功不等于应用已经响应。

## 验证

```powershell
npm run check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib tray_native::tests
# 只读观察当前原生托盘的可访问按钮、可见性；不自动打开应用
cargo test --manifest-path src-tauri/Cargo.toml --lib tray_native::tests::inspect_native_tray -- --ignored --nocapture
```

自动测试覆盖名称误匹配、多行提示归一化，以及真实 Win32 父子测试窗口接收的左右键、
双击事件与客户区坐标。测试窗口不注册系统托盘图标，也不操作用户应用。

重启新版后仍需实测：PixPin 回归、其他应用左右键/双击、Hub 常显与收纳两处、
Windows 原生任务栏显示/隐藏两种状态、连续点击、右键菜单可选中且不提前消失。
日志 `%TEMP%\window-hub-click-trace.log` 的 `native fallback` 和 `dispatch result` 可用于确认路径。
只读诊断在原生任务栏隐藏时返回零按钮，是可见性诊断，不代表交互验证通过。
名称变化、多实例同名或不提供可访问按钮的第三方托盘仍可能无法定位；应返回失败而不误点。
