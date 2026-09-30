---
name: window-hub-island-notify
description: >-
  Window Hub 灵动岛通知槽位 — hub.notify, urgency, TTL, rate limits, action buttons
  (start/end ordering), defaultActionId body click, hub.notify.onAction,
  coexistence with tray-attention. Use when implementing island notifications
  or plugins that push attention.
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
  /** 点横幅中部：有则 fire onAction(此 id) 后 dismiss；无则仅 dismiss（有 panel 则开面板） */
  defaultActionId?: string;
  actions?: NotifyActionInput[];
}): Promise<{ id: string }>

hub.notify.onAction((ev: {
  notifyId: string;
  actionId: string;
  data?: unknown;
}) => void): () => void
```

Rust：`hub_notify`（需 `notify` + slot `island.notify`）。  
点击按钮 / 中部 defaultAction → Host emit `island-notify-action` → 各表面转发 → `onAction`。

## 动作按钮规范（Host 强制）

布局由 Host 固定，插件**不能**自定义坐标：

| 规则 | 说明 |
|------|------|
| 排布 | **追加在文案后面**，与图标+文案组成一簇 **整体水平居中**（不再左右拉满） |
| `slot` | 仅 `start` / `end` 用于**排序**（先 start 后 end）；每槽最多 1 |
| 垂直 | 相对岛栏垂直居中 |
| 内容 | **恰好 2 个 Unicode 字符**的 `label`，**或** `iconPng`（互斥） |
| 背景 | 必填 `background`（安全 CSS 色）；建议半透明白以贴合黑岛 |
| 形状 | Host 胶囊（`border-radius: 999px`，高约 20） |
| 字号 | 约 **11px** / weight 650 |

```ts
actions: [
  { id: "done", slot: "start", label: "完成", background: "rgba(52,199,89,0.85)" },
  { id: "open", slot: "end", label: "打开", background: "rgba(255,255,255,0.2)" },
],
defaultActionId: "open", // 点中部 = 打开
```

`id` / `label` 文案由插件自定义；Host 只认 `actionId` 回传。可在 action 或 notify 上带 `data`。

校验实现：`src/plugins/notifyActions.ts`（`normalizeNotifyActions`）。

## 状态

| 路径 | 状态 |
|------|------|
| Popup / Panel / Shortcuts `hub.notify` | ✅ |
| 横幅文案后按钮簇（居中） | ✅ |
| `defaultActionId` 中部点击 | ✅ |
| `hub.notify.onAction` | ✅（快捷区常驻最稳） |
| 托盘 attention 横幅 | 无插件 actions（点条打开托盘） |

## 点击行为（Host 统一）

| 来源 | 点横幅中部（图标/文案区） | 点按钮 |
|------|---------------------------|--------|
| **插件** + `defaultActionId` | `onAction(defaultActionId)` → dismiss | `onAction` → dismiss |
| **插件** 无 default | dismiss → 若有 `island.panel` 则开面板 | `onAction` → dismiss |
| **托盘** attention | 唤起对应托盘应用 | — |

插件通知**禁止**走托盘 `invoke_tray_icon`。

## 其它规则

| 规则 | 说明 |
|------|------|
| Capability | `notify` + slot `island.notify` |
| 限流 | 默认 ≤ `maxPerMinute`（常 6） |
| TTL | 插件默认 **粘滞**（`ttlMs` 省略或 0）；托盘闪动同理 |
| 排队 / 叠层 | 插件通知**不合并**；多条可同时叠在岛下（最多约 6）；托盘同图标仍合并 |
| 点按钮 / default | 触发回调后 dismiss 该条 |

## 与其它临时态共存（Host）

主岛已被占用时，通知**不得盖住**栏内内容，改为主岛**下方独立胶囊**（顶边距 **4px**）：

| 冲突源 | 行为 |
|--------|------|
| Alt+空格搜索（`searchMode`） | 搜索栏 / 下拉保持；通知叠在下方 |
| 情景临时（正在播放等 `scenarioOwner`） | 歌词等摘要保持；通知叠在下方 |
| 下拉展开（`expanded` / `reveal`） | 面板保持；通知叠在面板下方 |

空闲常驻摘要时仍走岛内横幅落下（原 `is-notifying`）。冲突叠层时**强调描边只画在下方胶囊**，主岛不描边。实现：`src/App.tsx` `notifyStacked` / `.island-notify-stack` / `.bar-notify.is-stacked`。

叠层命中：`ignoreCursorEvents` 必须用 **整列** `notifyStackRef`（不能只认 primary）；`.island-notify-stack` z-index **高于** `.island-beam`，贴天气边时胶囊优先吃点击。

## Manifest

```json
{
  "slots": {
    "island.notify": { "priority": "active", "maxPerMinute": 6 }
  },
  "capabilities": ["notify"]
}
```
