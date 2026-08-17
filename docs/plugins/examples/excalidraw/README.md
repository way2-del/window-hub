# Excalidraw（快捷区）

基于 [excalidraw/excalidraw](https://github.com/excalidraw/excalidraw)（MIT）。

## 交互

1. 快捷区 **画板** → 较大**固定弹窗**（默认约 480×640）
2. **右缘拖拽**加宽；超过 `popupMaxWidth`（默认 720）→ 自动全屏系统窗口
3. 底部 **窗口化** → 手动切到系统标题栏窗口

```js
hub.popup.resize({ width, height })
hub.popup.openAsWindow({ windowedFullscreen: true })
```

画布写入 `hub.storage` 键 `scene`（两模式共用）。
