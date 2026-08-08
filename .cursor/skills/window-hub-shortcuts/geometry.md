# 快捷区几何契约

## 可用宽计算（伪代码）

```
settingsRight = settingsAnchor.left + settingsAnchor.width
dockLeft      = settingsRight + SHORTCUTS_LEFT_INSET   // inset = 12
islandLeft    = (viewportWidth - islandWidth) / 2     // 岛居中
dockRightMax  = islandLeft - SHORTCUTS_ISLAND_GAP     // gap 默认 24，最小 16
maxExpandWidth = max(0, dockRightMax - dockLeft)
```

岛 morph（展开变宽）时 `islandWidth` 变大 → `maxExpandWidth` 缩小；宿主须立即收紧展开条，避免压岛。

## getBounds 事件

- 首次挂载、窗口 resize、岛尺寸变化、设置标签显隐变化时 emit `shortcuts-bounds`
- 插件经 `hub.shortcuts.onBoundsChanged(cb)` 订阅（可选）；读写以 `getBounds()` 为准

## 滚动

- 仅横向；隐藏原生粗滚动条或用 Host 统一细条
- 滚轮纵向手势映射为横向 scroll（步进 `SHORTCUTS_SCROLL_STEP`）
- 可选左右箭头按钮，同步进
