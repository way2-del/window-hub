# 网页监测（官方插件）

打开网页（Host 托管 WebView2）、划定 DOM 元素并后台轮询文本变化；变更时推送灵动岛通知。

## 能力

| Capability / Slot | 用途 |
|-------------------|------|
| `webview` | `hub.webview.open` / `startPick` / `watch.*` |
| `notify` + `island.notify` | `hub.notify`；右侧「打开」走 `onAction` |
| `popup` + `shortcuts` | 列表 / 设置 / 日志 UI |
| `storage` | 监测任务、扫描日志、通知模板 |

Cookie / 登录态保存在 `%APPDATA%/window-hub/webview-profiles/{pluginId}/`，与交互浏览共用。

## 使用

1. 快捷区点开弹窗 → **添加**
2. 填网址 → **打开** → **划定** 元素，或点 **整页**
3. 设间隔（≥30s）→ **保存监测**
4. 列表点 **编辑** 可改标题 / 网址 / 选择器 / 间隔；**保存修改**会丢弃旧基线、刷新预览，下次成功扫描记为新基线；开关状态保留
5. 点 **日志** 进入二级页；顶栏 **日志** 可看全部
6. 顶栏 **设置**：配置通知标题/正文模板（如 `{title}更新了`）
7. 内容变化后灵动岛横幅；点中部或「打开」都会打开对应监测网页

## 通知模板变量

| 变量 | 含义 |
|------|------|
| `{title}` / `{name}` | 监测项名称 |
| `{text}` | 本次采样文本 |
| `{url}` | 网址 |
| `{selector}` | CSS 选择器 |

默认：标题 `{title}`，正文 `{title}更新了`。

## 注意

- **快捷区条**：图标为 16×16 居中方正 glyph + 简称「监测」。防抖：插件与 Host `ShortcutsPluginStrip` 都只报 `scrollWidth`、缓存上次宽度，禁止 `resize`/`clientWidth` 反馈环。详见 [插件开发工作流 · 快捷区条](../../development.md#shortcuts-strip-jitter)
- **关掉浏览窗不影响监测**：后台用独立隐藏 WebView 按保存的 URL/选择器轮询；浏览窗只用于登录与划定
- **岛通知只由快捷区发出**（常驻表面）；弹窗只记扫描日志，避免与快捷区各发一次叠成多条
- **及时、不堆积**：用 Host 事件 `atMs` 拼 `changeId`（`watchId@atMs`）做唯一去重——同一扫描只推一次；新 `atMs` 立即推。`hub.notify` 的 `data.changeId` / `data.atMs` 可核对；控制台有 `notify` / `skip duplicate` 日志
- **点横幅中部**：`defaultActionId: "open"` → 打开对应网页；无 default 时 Host 仅 dismiss
- 删除监测项 / 清空日志均有应用内二次确认（不用 `confirm`）
- 强风控 / 验证码站点仍可能拦截自动化导航
- Hub 重启后由快捷区条 / 弹窗重新 `watch.start` 登记
- 划定若失败可改用整页，或手填 CSS 选择器
- **XML / XSLT 页**：WebView2 会提示 XSLT 即将移除；整页（`body`）监测可能把浏览器提示当成内容变化。尽量划定真正业务节点，不要用整页
