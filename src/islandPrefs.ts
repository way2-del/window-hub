/** 灵动岛偏好（设置窗 ↔ 主顶栏通过 SQLite + Tauri 事件同步） */

import { invoke } from "@tauri-apps/api/core";

/** 下拉展开内容：`plugin:{id}`（legacy weather|mirror 会迁移） */
export type PullContent = string;

export type IslandPrefs = {
  /** 闲置后自动沉浸（透明融顶栏） */
  autoImmerse: boolean;
  /** 无操作多久后沉浸（秒） */
  immerseIdleSec: number;
  /** 下拉岛默认展示内容 */
  pullContent: PullContent;
  /**
   * @deprecated 由 barPriority[0] 同步；设置 UI 请改 barPriority。
   */
  barResident: string;
  /**
   * 岛栏内容竞选顺序（高 → 低）。通知横幅始终最上；中转站临时层另算。
   * 列表外的已启用 island.bar 插件会按 slot.order 追加在末尾。
   */
  barPriority: string[];
  /** 托盘闪动时在岛上弹出消息提示 */
  msgNotify: boolean;
  /** 无具体内容时的默认文案 */
  msgNotifyText: string;
  /** 提示展示时长（秒）——仅作插件 notify 默认 TTL；托盘提示仍常驻 */
  msgNotifySec: number;
  /** 调节岛栏音量后播放系统提示音 */
  volumePreviewSound: boolean;
  /** 顶栏磨砂半透明（默认关，实心取色） */
  topbarFrost: boolean;
};

const LS_AUTO = "wh-island-auto-immerse";
const LS_IDLE = "wh-island-immerse-idle-sec";
const LS_PULL = "wh-island-pull-content";
const LS_MSG = "wh-island-msg-notify";
const LS_MSG_TEXT = "wh-island-msg-notify-text";
const LS_MSG_SEC = "wh-island-msg-notify-sec";
export const ISLAND_PREFS_KEYS = [
  LS_AUTO,
  LS_IDLE,
  LS_PULL,
  LS_MSG,
  LS_MSG_TEXT,
  LS_MSG_SEC,
  "wh-island-staging-panel-w",
] as const;

/** 插件面板宽度可选值（与 transfer-station settings.panelWidth 对齐） */
export const STAGING_PANEL_W_OPTIONS = [440, 520, 560, 640, 720] as const;
export const STAGING_PANEL_W_MIN = 440;
export const STAGING_PANEL_W_MAX = 720;
export const STAGING_PANEL_W_DEFAULT = 560;
/** 插件面板高度可选值（与 transfer-station settings.panelHeight 对齐；默认保持原中转站高度） */
export const STAGING_PANEL_H_OPTIONS = [120, 136, 152, 168, 184] as const;
export const STAGING_PANEL_H_MIN = 120;
export const STAGING_PANEL_H_MAX = 184;
export const STAGING_PANEL_H_DEFAULT = 152;
/** @deprecated 使用 STAGING_PANEL_H_DEFAULT */
export const STAGING_PANEL_H = STAGING_PANEL_H_DEFAULT;

/** 官方岛栏默认竞选顺序：有歌词 > 有待办 > 天气 */
export const DEFAULT_BAR_PRIORITY = [
  "com.window-hub.lyrics",
  "com.window-hub.todo",
  "com.window-hub.weather",
] as const;

const DEFAULTS: IslandPrefs = {
  autoImmerse: true,
  immerseIdleSec: 8,
  /** Weather is a plugin; migrate legacy "weather"|"mirror" in parsePullContent */
  pullContent: "plugin:com.window-hub.weather",
  barResident: "com.window-hub.lyrics",
  barPriority: [...DEFAULT_BAR_PRIORITY],
  msgNotify: true,
  msgNotifyText: "收到一条消息",
  msgNotifySec: 4,
  volumePreviewSound: true,
  topbarFrost: false,
};

const IDLE_MIN = 2;
const IDLE_MAX = 300;
const MSG_SEC_MIN = 2;
const MSG_SEC_MAX = 30;

let cache: IslandPrefs = { ...DEFAULTS };
let hydrated = false;

function clampIdle(sec: number) {
  if (!Number.isFinite(sec)) return DEFAULTS.immerseIdleSec;
  return Math.max(IDLE_MIN, Math.min(IDLE_MAX, Math.round(sec)));
}

function clampMsgSec(sec: number) {
  if (!Number.isFinite(sec)) return DEFAULTS.msgNotifySec;
  return Math.max(MSG_SEC_MIN, Math.min(MSG_SEC_MAX, Math.round(sec)));
}

export function clampStagingPanelW(w: number): number {
  if (!Number.isFinite(w)) return STAGING_PANEL_W_DEFAULT;
  const rounded = Math.round(w);
  const nearest = STAGING_PANEL_W_OPTIONS.reduce((best, cur) =>
    Math.abs(cur - rounded) < Math.abs(best - rounded) ? cur : best,
  );
  return Math.max(STAGING_PANEL_W_MIN, Math.min(STAGING_PANEL_W_MAX, nearest));
}

export function clampStagingPanelH(h: number): number {
  if (!Number.isFinite(h)) return STAGING_PANEL_H_DEFAULT;
  const rounded = Math.round(h);
  const nearest = STAGING_PANEL_H_OPTIONS.reduce((best, cur) =>
    Math.abs(cur - rounded) < Math.abs(best - rounded) ? cur : best,
  );
  return Math.max(STAGING_PANEL_H_MIN, Math.min(STAGING_PANEL_H_MAX, nearest));
}

function parsePullContent(raw: string | null): PullContent {
  if (!raw) return DEFAULTS.pullContent;
  // Legacy Host builtins → official plugins
  if (raw === "weather") return "plugin:com.window-hub.weather";
  if (raw === "mirror") return "plugin:com.window-hub.mirror";
  if (raw.startsWith("plugin:")) return raw;
  return DEFAULTS.pullContent;
}

function parseBarResident(raw: string | null | undefined): string {
  if (raw == null) return DEFAULTS.barResident;
  const t = String(raw).trim();
  if (!t || t === "none" || t === "off") return "";
  // Accept legacy pullContent-style ids
  if (t.startsWith("plugin:")) return t.slice("plugin:".length);
  return t;
}

function parseBarPriority(raw: unknown, legacyResident?: string): string[] {
  const out: string[] = [];
  if (Array.isArray(raw)) {
    for (const item of raw) {
      const id = String(item ?? "").trim();
      if (!id || out.includes(id)) continue;
      out.push(id);
    }
  }
  if (!out.length) {
    const r = parseBarResident(legacyResident);
    if (r) out.push(r);
  }
  return out;
}

/** 把新插件按 DEFAULT_BAR_PRIORITY 相对位置插入（已有项不重排）。 */
function insertByPreferredOrder(ordered: string[], id: string, preferred: readonly string[]) {
  if (ordered.includes(id)) return;
  const prefIdx = preferred.indexOf(id);
  if (prefIdx < 0) {
    ordered.push(id);
    return;
  }
  let after = -1;
  for (let i = 0; i < ordered.length; i++) {
    const oi = preferred.indexOf(ordered[i]!);
    if (oi >= 0 && oi < prefIdx) after = i;
  }
  if (after >= 0) {
    ordered.splice(after + 1, 0, id);
    return;
  }
  let before = -1;
  for (let i = 0; i < ordered.length; i++) {
    const oi = preferred.indexOf(ordered[i]!);
    if (oi > prefIdx) {
      before = i;
      break;
    }
  }
  if (before >= 0) {
    ordered.splice(before, 0, id);
    return;
  }
  ordered.push(id);
}

/**
 * 与已启用 island.bar 插件对齐：保留用户顺序，剔除失效；
 * 新插件按 DEFAULT_BAR_PRIORITY（歌词 > 待办 > 天气）相对位置插入。
 */
export function mergeBarPriority(
  saved: string[] | undefined,
  availableIds: string[],
  legacyResident?: string,
): string[] {
  const avail = availableIds.map((id) => String(id ?? "").trim()).filter(Boolean);
  const availSet = new Set(avail);
  const base = parseBarPriority(saved, legacyResident).filter((id) => availSet.has(id));
  const ordered = [...base];
  for (const id of avail) {
    insertByPreferredOrder(ordered, id, DEFAULT_BAR_PRIORITY);
  }
  return ordered;
}

function parseMsgText(raw: string | null): string {
  const t = (raw ?? "").trim();
  return t || DEFAULTS.msgNotifyText;
}

function readLegacyLocalStorage(): IslandPrefs | null {
  try {
    const hasAny = ISLAND_PREFS_KEYS.some((k) => localStorage.getItem(k) != null);
    if (!hasAny) return null;
    const autoRaw = localStorage.getItem(LS_AUTO);
    const idleRaw = localStorage.getItem(LS_IDLE);
    const pullRaw = localStorage.getItem(LS_PULL);
    const msgRaw = localStorage.getItem(LS_MSG);
    const msgTextRaw = localStorage.getItem(LS_MSG_TEXT);
    const msgSecRaw = localStorage.getItem(LS_MSG_SEC);
    return {
      autoImmerse: autoRaw == null ? DEFAULTS.autoImmerse : autoRaw === "1" || autoRaw === "true",
      immerseIdleSec: idleRaw == null ? DEFAULTS.immerseIdleSec : clampIdle(Number(idleRaw)),
      pullContent: parsePullContent(pullRaw),
      barResident: DEFAULTS.barResident,
      barPriority: [...DEFAULTS.barPriority],
      msgNotify: msgRaw == null ? DEFAULTS.msgNotify : msgRaw === "1" || msgRaw === "true",
      msgNotifyText: parseMsgText(msgTextRaw),
      msgNotifySec: msgSecRaw == null ? DEFAULTS.msgNotifySec : clampMsgSec(Number(msgSecRaw)),
      volumePreviewSound: DEFAULTS.volumePreviewSound,
      topbarFrost: DEFAULTS.topbarFrost,
    };
  } catch {
    return null;
  }
}

function clearLegacyLocalStorage() {
  try {
    for (const k of ISLAND_PREFS_KEYS) localStorage.removeItem(k);
  } catch {
    /* noop */
  }
}

function mergePrefs(prev: IslandPrefs, partial: Partial<IslandPrefs>): IslandPrefs {
  const barPriority =
    partial.barPriority != null
      ? parseBarPriority(partial.barPriority, partial.barResident ?? prev.barResident)
      : parseBarPriority(prev.barPriority, prev.barResident);
  const barResident =
    partial.barResident != null
      ? parseBarResident(partial.barResident)
      : barPriority[0] ?? prev.barResident;
  return {
    autoImmerse: partial.autoImmerse ?? prev.autoImmerse,
    immerseIdleSec:
      partial.immerseIdleSec != null ? clampIdle(partial.immerseIdleSec) : prev.immerseIdleSec,
    pullContent: partial.pullContent != null ? parsePullContent(partial.pullContent) : prev.pullContent,
    barResident: barPriority[0] ?? barResident,
    barPriority,
    msgNotify: partial.msgNotify ?? prev.msgNotify,
    msgNotifyText:
      partial.msgNotifyText != null ? parseMsgText(partial.msgNotifyText) : prev.msgNotifyText,
    msgNotifySec:
      partial.msgNotifySec != null ? clampMsgSec(partial.msgNotifySec) : prev.msgNotifySec,
    volumePreviewSound: partial.volumePreviewSound ?? prev.volumePreviewSound,
    topbarFrost: partial.topbarFrost ?? prev.topbarFrost,
  };
}

/** Sync snapshot（hydrate 前为默认值） */
export function getIslandPrefs(): IslandPrefs {
  return cache;
}

/** 启动时从 DB 拉取；若仍有 localStorage 则一次性迁入后清除 */
export async function hydrateIslandPrefs(): Promise<IslandPrefs> {
  try {
    const legacy = readLegacyLocalStorage();
    if (legacy) {
      cache = await invoke<IslandPrefs>("set_island_prefs", { prefs: legacy });
      clearLegacyLocalStorage();
    } else {
      const raw = await invoke<IslandPrefs>("get_island_prefs");
      cache = mergePrefs(DEFAULTS, raw);
    }
  } catch {
    cache = { ...DEFAULTS };
  }
  hydrated = true;
  window.dispatchEvent(new Event("wh-island-prefs"));
  return cache;
}

export async function setIslandPrefs(partial: Partial<IslandPrefs>): Promise<IslandPrefs> {
  if (!hydrated) {
    await hydrateIslandPrefs();
  }
  const next = mergePrefs(cache, partial);
  try {
    cache = await invoke<IslandPrefs>("set_island_prefs", { prefs: next });
  } catch {
    cache = next;
  }
  window.dispatchEvent(new Event("wh-island-prefs"));
  return cache;
}

/** Apply prefs from Tauri event payload (cross-window). */
export function applyIslandPrefsSnapshot(prefs: IslandPrefs): IslandPrefs {
  cache = mergePrefs(DEFAULTS, prefs);
  window.dispatchEvent(new Event("wh-island-prefs"));
  return cache;
}

export function subscribeIslandPrefs(onChange: (prefs: IslandPrefs) => void): () => void {
  const emit = () => onChange(getIslandPrefs());
  window.addEventListener("wh-island-prefs", emit);
  return () => {
    window.removeEventListener("wh-island-prefs", emit);
  };
}
