---
name: window-hub-island-notify
description: >-
  Window Hub 灵动岛通知槽位 — island.notify slot、urgency/TTL/限流、与 tray-attention
  共存。当前插件 iframe 未注入 hub.notify（仅 Host hubNotify）。改通知通路时用。
---

# 灵动岛通知（island.notify）

Slot：`island.notify`。Host 拥有动画与排队；**插件禁止自建 toast 窗**。

## 当前实现状态（重要）

| 路径 | 状态 |
|------|------|
| Host 内 `hubNotify(manifest, args)` → 事件 `island-notify` | ✅ `src/plugins/notifyApi.ts` |
| Popup / Panel / Shortcuts 注入 `hub.notify` | ❌ **未实现** |
| `onNotifyAction` 回插件 | ❌ **未实现** |

写**第三方插件包**时不要调用 `hub.notify`。宿主内置或后续注入完成前，通知仅能从 Host 代码路径发出。

## 目标 API（落地后）

```ts
hub.notify({
  title: string;
  body?: string;
  iconPng?: string;
  urgency?: "passive" | "active" | "critical";
  ttlMs?: number;
  actions?: { id: string; label: string }[];
}): Promise<{ id: string }>
```

## 规则（Host 总线已遵守）

| 规则 | 说明 |
|------|------|
| Capability | `notify` + slot `island.notify` |
| 限流 | 默认 ≤ `maxPerMinute`（常 6）；`critical` 可插队仍计数 |
| 排队 | 同时一条横幅 |
| 共存 | 与 `tray-attention` 共用通路 |
| 岛展开时 | 默认不抢横幅 |

## Manifest

```json
{
  "slots": {
    "island.notify": { "priority": "active", "maxPerMinute": 6 }
  },
  "capabilities": ["notify"]
}
```

## 与快捷区

瞬时注意力 → 岛通知；常驻入口 → 快捷区。
