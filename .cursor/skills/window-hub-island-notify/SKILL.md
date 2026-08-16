---
name: window-hub-island-notify
description: >-
  Window Hub 灵动岛通知槽位 — hub.notify, urgency, TTL, rate limits, action buttons
  (start/end only), hub.notify.onAction callbacks, coexistence with tray-attention.
  Use when implementing island notifications or plugins that push attention.
---

# 灵动岛通知（island.notify）

Slot：`island.notify`。Host 拥有动画与排队；**插件禁止自建 toast 窗**。

## API

```ts
hub.notify({
  title: string;
  body?: string;
  iconPng?: string;
  urgency?: "passive" | "active" | "critical";
  ttlMs?: number;
  data?: unknown; // 整条通知级回传（按钮未带 data 时用）
  actions?: NotifyActionInput[];
}): Promise<{ id: string }>

hub.notify.onAction((ev: {
  notifyId: string;
  actionId: string;
  data?: unknown;
}) => void): () => void
```

Rust：`hub_notify`（需 `notify` + slot `island.notify`）。  
点击按钮 → Host emit `island-notify-action` → 各表面转发 → `onAction`。

## 动作按钮规范（Host 强制）

布局由 Host 固定，插件**不能**自定义坐标：

| 规则 | 说明 |
|------|------|
| 位置 | 仅 `slot: "start"`（文案左侧）或 `"end"`（文案右侧） |
| 每槽数量 | **最多 1**；非法/重复槽位丢弃 |
| 垂直 | 相对岛栏垂直居中 |
| 内容 | **恰好 2 个 Unicode 字符**的 `label`，**或** `iconPng`（互斥） |
| 背景 | 必填 `background`（安全 CSS 色：`#rgb` / `#rrggbb` / `rgb()` / `rgba()` / `hsl()` / `hsla()`） |
| 形状 | Host 大圆角矩形（`border-radius: 11px`，高 22） |
| 字号 | 与岛栏文案相同（**12px** / weight 600） |

```ts
actions: [
  { id: "done", slot: "start", label: "完成", background: "#34c759" },
  { id: "later", slot: "end", label: "稍后", background: "rgba(255,255,255,0.22)" },
]
```

`id` / `label` 文案由插件自定义；Host 只认 `actionId` 回传。可在 action 或 notify 上带 `data`。

校验实现：`src/plugins/notifyActions.ts`（`normalizeNotifyActions`）。

## 状态

| 路径 | 状态 |
|------|------|
| Popup / Panel / Shortcuts `hub.notify` | ✅ |
| 横幅 start/end 按钮 UI | ✅ |
| `hub.notify.onAction` | ✅（快捷区常驻最稳） |
| 托盘 attention 横幅 | 无插件 actions（点条打开托盘） |

## 点击行为（Host 统一，所有插件）

| 来源 | 点横幅中部 | 点 start/end 按钮 |
|------|------------|-------------------|
| **插件** `hub.notify` | dismiss → 若有 `island.panel` 则下拉打开该插件面板 | `hub.notify.onAction` → dismiss |
| **托盘** attention | 唤起对应托盘应用（旧行为） | — |

插件通知**禁止**走托盘 `invoke_tray_icon`。无 `island.panel` 时点中部仅 dismiss。

## 其它规则

| 规则 | 说明 |
|------|------|
| Capability | `notify` + slot `island.notify` |
| 限流 | 默认 ≤ `maxPerMinute`（常 6） |
| 排队 | 同时一条横幅 |
| 点按钮 | 触发回调后 dismiss |

## 与其它临时态共存（Host）

主岛已被占用时，通知**不得盖住**栏内内容，改为主岛**下方独立胶囊**（顶边距 **4px**）：

| 冲突源 | 行为 |
|--------|------|
| Alt+空格搜索（`searchMode`） | 搜索栏 / 下拉保持；通知叠在下方 |
| 情景临时（正在播放等 `scenarioOwner`） | 歌词等摘要保持；通知叠在下方 |
| 下拉展开（`expanded` / `reveal`） | 面板保持；通知叠在面板下方 |

空闲常驻摘要时仍走岛内横幅落下（原 `is-notifying`）。冲突叠层时**强调描边只画在下方胶囊**，主岛不描边。实现：`src/App.tsx` `notifyStacked` / `.island-notify-stack` / `.bar-notify.is-stacked`。

## Manifest

```json
{
  "slots": {
    "island.notify": { "priority": "active", "maxPerMinute": 6 }
  },
  "capabilities": ["notify"]
}
```
