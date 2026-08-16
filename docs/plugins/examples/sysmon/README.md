# 系统监控（官方示例）

快捷区常驻风扇：转速随 **有效温度** `max(CPU, GPU)` 加快；点击打开详情弹窗。

## 能力

| Capability | 用途 |
|------------|------|
| `system.monitor` | `hub.sysmon.snapshot()`（基座 Rust） |
| `shortcuts` / `popup` / `storage` | 快捷区 + 弹窗 + 设置 |

## 安装

设置 → 插件市场 → **系统监控** → 导入示例，或「添加开发目录」指向本目录。

## 说明

- CPU / 内存 / 磁盘：本机 `sysinfo` 采集。
- 温度：Windows 为 best-effort（传感器 + 可选 NVIDIA NVML）；无读数时风扇慢转，弹窗显示「暂无」。
