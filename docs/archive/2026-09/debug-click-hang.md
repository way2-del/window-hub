# 排查：插件 / SQLite / 资源 / 更早 commit

> 历史排障记录，以下“已做/请验证”是当时上下文，不是当前执行指令。监控工具现位于 `scripts/dev/monitor-resources.ps1`，新日志写 `workspace/logs/`。

## 已做

1. **禁用全部插件**（备份在 `plugins\registry.json.bak-before-disable-*`）  
   原先启用：weather, mirror, transfer-station, now-playing, file-search, file-search__dev  
2. **SQLite**：`%APPDATA%\window-hub\window-hub.db`  
   - DB ~236KB，**WAL 曾达 ~4MB**（异常膨胀，常见于崩溃/未 checkpoint）  
   - 进程杀掉后文件未锁；启动时增加 `busy_timeout` + `wal_checkpoint(TRUNCATE)`  
3. **资源监测**：`docs/monitor-resources.ps1` → `%TEMP%\window-hub-resource.log`

## 最新卡死日志（非仅收起）

```
status-menu build DONE
open_settings build DONE
HUNG
```

截图标题是 **「灵动岛设置 (未响应)」** + 插件市场页 → 与连续创建 WebView（菜单→设置）同类问题。已：开设置前关掉菜单弹窗、加长 delay、延后 show/focus。

## 往前对照 commit（不只最新）

| Commit | 内容 | 与卡死关系 |
|--------|------|------------|
| `860e29d` | Fix click Not Responding | 曾修好一轮 |
| `d065cf1` | **插件市场 UI** 大改 | 设置里插件页起点 |
| `e5ec890` | settings Mica / hotkey UI | 设置窗材质 |
| `c1579a6` | ambient / boot | 收起仍用 setSize |
| **`f9aefde`** | **Win32 resize_main_island** | 岛收起卡死主因（宿主，非插件代码） |

**结论**：岛收起卡死主因是宿主 `f9aefde` 的 SetWindowPos 路径；设置卡死另有「连续 WebView create」路径。插件会加重（now-playing 轮询、Everything），但不是唯一根因——先全禁用做 A/B。

## 请你验证

```powershell
# 终端1：资源监测
powershell -File d:\project\window-hub\docs\monitor-resources.ps1

# 终端2：已用 build-and-run 启动后
# 1) 下拉/收起灵动岛  2) 打开设置→插件市场
# 若仍 HUNG：把 %TEMP%\window-hub-click-trace.log 与 window-hub-resource.log 尾部发我
```

恢复插件：把 `registry.json.bak-before-disable-*` 拷回 `registry.json`。
