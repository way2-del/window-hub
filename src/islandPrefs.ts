/** 灵动岛偏好（设置窗 ↔ 主顶栏通过 SQLite + Tauri 事件同步） */

import { invoke } from "@tauri-apps/api/core";

/** 下拉展开内容：内置 weather|mirror，或 `plugin:{id}` */
export type PullContent = string;

export type IslandPrefs = {
  /** 闲置后自动沉浸（透明融顶栏） */
  autoImmerse: boolean;
  /** 无操作多久后沉浸（秒） */
  immerseIdleSec: number;
  /** 下拉岛默认展示内容 */
  pullContent: PullContent;
  /** 托盘闪动时在岛上弹出消息提示 */
  msgNotify: boolean;
  /** 无具体内容时的默认文案 */
  msgNotifyText: string;
  /** 提示展示时长（秒）——仅作插件 notify 默认 TTL；托盘提示仍常驻 */
  msgNotifySec: number;
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

const DEFAULTS: IslandPrefs = {
  autoImmerse: true,
  immerseIdleSec: 8,
  pullContent: "weather",
  msgNotify: true,
  msgNotifyText: "收到一条消息",
  msgNotifySec: 4,
};

const IDLE_MIN = 2;
const IDLE_MAX = 300;
const MSG_SEC_MIN = 2;
const MSG_SEC_MAX = 30;

const BUILTIN_PULL = new Set(["weather", "mirror"]);

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
  if (BUILTIN_PULL.has(raw) || raw.startsWith("plugin:")) return raw;
  return DEFAULTS.pullContent;
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
      msgNotify: msgRaw == null ? DEFAULTS.msgNotify : msgRaw === "1" || msgRaw === "true",
      msgNotifyText: parseMsgText(msgTextRaw),
      msgNotifySec: msgSecRaw == null ? DEFAULTS.msgNotifySec : clampMsgSec(Number(msgSecRaw)),
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
  return {
    autoImmerse: partial.autoImmerse ?? prev.autoImmerse,
    immerseIdleSec:
      partial.immerseIdleSec != null ? clampIdle(partial.immerseIdleSec) : prev.immerseIdleSec,
    pullContent: partial.pullContent != null ? parsePullContent(partial.pullContent) : prev.pullContent,
    msgNotify: partial.msgNotify ?? prev.msgNotify,
    msgNotifyText:
      partial.msgNotifyText != null ? parseMsgText(partial.msgNotifyText) : prev.msgNotifyText,
    msgNotifySec:
      partial.msgNotifySec != null ? clampMsgSec(partial.msgNotifySec) : prev.msgNotifySec,
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
      cache = await invoke<IslandPrefs>("get_island_prefs");
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
