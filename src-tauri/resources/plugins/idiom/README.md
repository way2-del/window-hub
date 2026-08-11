# 成语（官方插件）

左侧**快捷区**自画 chip 显示随机成语。数据来自 `http://43.139.23.203:8765/api/random`。

**不占用岛栏**。拼音使用同目录 vendored 库 [`pinyin-pro`](https://www.npmjs.com/package/pinyin-pro)（`pinyin-pro.min.js`），**默认带声调符号**（如 `chéng yǔ`）。

| 交互 | 行为 |
|------|------|
| 2×2 管理钮 | `manage=custom` → 打开历史弹窗 |
| 快捷区文案 | `word` 成语 |
| 悬停 | Host tip（`showTip` / 自动拦截 `title`）= 带调拼音 + 释义；材质同插件弹窗 |
| 点击成语文案 | 立即拉取下一条 |
| 设置「自动切换间隔」 | 定时刷新；选「仅手动点击」则只靠点击 |
| 历史弹窗 | 最近 **200** 条；每页 8 条；点击看详情（词 / 拼音 / 释义 / 时间） |

## 存储（`hub.storage`）

| Key | 内容 |
|-----|------|
| `cache` | 当前展示项 `{ item, savedAt }` |
| `history` | `{ version: 1, items: [{ id, word, meaning, pinyin, seenAt }] }` 最多 200 |

## Manifest 要点

```json
{
  "entry": { "shortcuts": "shortcuts.html", "popup": "popup.html" },
  "slots": {
    "shortcuts": {
      "manage": "custom",
      "action": "popup.open"
    }
  },
  "capabilities": ["storage", "network", "shortcuts", "popup"]
}
```

Host 优先注入同目录 `pinyin-pro.min.js`（其次兼容旧的 `pinyinlite.min.js`）。
