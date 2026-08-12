# tray-visual-smoke

本机冒烟：截取 Window Hub 托盘/系统/插件弹窗，拒绝「整块近白空壳」（white zombie），并用 OCR 核对中文文案。

## 前置

1. 先启动 Window Hub（建议等 1–2 秒让 `warm_popup_windows` 完成）。
2. OCR 二选一：
   - **Tesseract**：`tesseract` 在 PATH，语言包 `chi_sim`
   - **WinRT**：Windows 设置 → 时间和语言 → 语言 → 中文（简体）→ 选项 → **光学字符识别**；工具会调 `ocr-winrt.ps1`

## 用法

```powershell
cd D:\workProject\win-hub\tools\tray-visual-smoke

# 列出当前可见的候选弹窗
cargo run --release -- list

# 先点岛栏收纳箭头，再立刻检查（标题「已收纳」）
cargo run --release -- check --expect 已收纳 --wait-ms 2000 --out-dir .\out

# 系统面板
cargo run --release -- check --expect 系统面板 --wait-ms 2000

# 仅探测 OCR 后端
cargo run --release -- ocr-probe
```

## 断言含义

| 检查 | 预算 / 阈值 |
|------|-------------|
| 弹窗出现（`wait` 内找到标题） | warm 重开目标 ≤ **300ms**（超时只 WARN；冷 WebView2 首建会更慢） |
| 中心区域近白像素比 | 默认 ≤ **0.92**（超则判 white zombie） |
| 窗口标题 | 必须包含 `--expect`（主断言，不依赖 OCR） |
| OCR（可选） | 有 Tesseract/`chi_sim` 或 WinRT 中文 OCR 时额外核对画面文字 |

本机若未装 OCR，`ocr-probe` 会 WARN 并以 0 退出；`check` 仍以**标题 + 近白比**为准。优先安装：

```text
winget install UB-Mannheim.TesseractOCR
# 并确保 chi_sim.traineddata 可用
```

`invoke` 往返 ≤80ms、左键后岛栏仍 TOPMOST 等，需在运行中的 Host 内测；本工具专注「看得见的」弹窗回归。

## MyDockFinder

同机建议各跑一轮：

1. **开着 MyDockFinder**：点收纳箭头 → `check --expect 已收纳`
2. **退出 MyDockFinder**：重复一次

若仅在开着 Dock 时出现 Z-order 闪烁/菜单被盖，优先查 Host `topmost::yield_for`（左键不应再 yield）与右键菜单重定位，而不是 OCR。

## 与本轮大修的对应关系

- trayhook 瘦热路径 → QQ/微信闪动时顶栏应更顺（本工具不直接测）
- 弹窗 reveal 门闸 → `check` 不应再命中 white zombie
- 点击 Z-order → MyDockFinder 开/关对比主观手感
