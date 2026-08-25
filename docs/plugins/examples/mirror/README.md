# 镜子（官方示例）

仅 `island.panel`：灵动岛下拉时显示摄像头预览。

- 无岛栏、无快捷区、无 Host 硬编码设置
- Host iframe 需 `allow="camera"`
- 若曾点「拒绝」：面板内点「重新授权」重置 WebView2 权限；或「系统设置」打开 Windows 相机隐私
- 安装：首次启动自动 ensure，或设置 → 插件市场 →「导入示例：镜子」
- capability：`island.panel` + `media.camera`
