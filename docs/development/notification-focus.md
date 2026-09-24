# 托盘通知的前台确认

只有主窗口正在呈现的托盘通知启用 `useTrayNotificationFocus`。hook 先订阅 `tray-notification-viewed`，再通过 `watch_tray_notification` 注册通知来源 HWND；关闭、切换通知或卸载时撤销。每次订阅使用独立 token，晚到事件不能关闭新通知，旧清理不能撤销新监控。插件通知不参与。

Rust `win32/notification_focus.rs` 在注册时记录来源窗口 PID。复用 ambient watcher 的 80ms 前台 HWND 探测，只有监控存在时才检查前台 PID，并复核来源 HWND 仍属于原 PID。匹配同一前台窗口连续至少 240ms 后，一次性通知主窗口调用已有 dismissMsgBanner，关闭横幅并 clear_tray_attention。quiet 或捕获耗时可能延后回调，这不是严格实时承诺。

无通知时仅一次 AtomicBool 读取。有通知时每个现有周期最多两次 GetWindowThreadProcessId 和一次 try_lock；没有截图、进程枚举、OpenProcess、SQLite、独立线程或前端定时 IPC。锁被占用时跳过当次检查，不阻塞采样线程。

含义是“已切到来源应用”，不代表读取了某条聊天。只匹配同 PID，不按标题/可执行文件名或不可靠的父子进程关系猜测。主界面与托盘归属不同进程的应用会保留通知，仍可手动关闭。排队通知在显示时逐个检查。通知样式及插件 API 不变。

验证：Rust 测试覆盖停留阈值、中途换窗/离开、无效窗口、实际 HWND→PID 校验、匹配后一次性解除；前端通过通用检查。真实微信快捷键→横幅消失仍需有实际托盘消息时验证，不能由单元测试代替。
