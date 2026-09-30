/** 顶栏（系统菜单栏）可见性偏好 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  normalizeChromeChipOrder,
  type ChromeChipId,
} from "./features/chrome/chromeChipOrder";
import {
  applyBarHeightCss,
  DEFAULT_BAR_H,
  normalizeBarHeight,
  setLiveBarHeight,
} from "./features/chrome/barHeight";

export type ChromePrefs = {
  /** 托盘常驻图标（顶栏） */
  showTray: boolean;
  /** 托盘下拉列表（▾ 弹出） */
  showTrayMenu: boolean;
  /** WLAN / 以太网 */
  showWifi: boolean;
  /** 系统时间日期 */
  showClock: boolean;
  /** 输入法 / 语言 */
  showIme: boolean;
  /** 控制中心 */
  showControlCenter: boolean;
  /** 右侧系统芯片 Ctrl+拖 顺序 */
  chipOrder: ChromeChipId[];
  /** 顶栏 / 岛栏折叠高度（逻辑 px，24–40） */
  barHeight: number;
};

const DEFAULTS: ChromePrefs = {
  showTray: true,
  showTrayMenu: true,
  showWifi: true,
  showClock: true,
  showIme: true,
  showControlCenter: true,
  chipOrder: normalizeChromeChipOrder(null),
  barHeight: DEFAULT_BAR_H,
};

let cache: ChromePrefs = { ...DEFAULTS, chipOrder: [...DEFAULTS.chipOrder] };
let hydrated = false;
const listeners = new Set<(p: ChromePrefs) => void>();

function normalize(raw: Partial<ChromePrefs> | null | undefined): ChromePrefs {
  return {
    showTray: raw?.showTray ?? true,
    showTrayMenu: raw?.showTrayMenu ?? true,
    showWifi: raw?.showWifi ?? true,
    showClock: raw?.showClock ?? true,
    showIme: raw?.showIme ?? true,
    showControlCenter: raw?.showControlCenter ?? true,
    chipOrder: normalizeChromeChipOrder(raw?.chipOrder),
    barHeight: normalizeBarHeight(raw?.barHeight ?? DEFAULT_BAR_H),
  };
}

function applyLiveBar(p: ChromePrefs) {
  setLiveBarHeight(p.barHeight);
  applyBarHeightCss(p.barHeight);
}

function notify(p: ChromePrefs) {
  applyLiveBar(p);
  for (const fn of listeners) {
    try {
      fn(p);
    } catch (e) {
      console.error(e);
    }
  }
}

export function getChromePrefs(): ChromePrefs {
  return { ...cache, chipOrder: [...cache.chipOrder] };
}

export function subscribeChromePrefs(fn: (p: ChromePrefs) => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

export async function hydrateChromePrefs(): Promise<ChromePrefs> {
  try {
    const p = await invoke<ChromePrefs>("get_chrome_prefs");
    cache = normalize(p);
  } catch (e) {
    console.error(e);
    cache = { ...DEFAULTS, chipOrder: [...DEFAULTS.chipOrder] };
  }
  hydrated = true;
  notify(cache);
  return getChromePrefs();
}

export async function setChromePrefs(
  patch: Partial<ChromePrefs>,
): Promise<ChromePrefs> {
  const next = normalize({ ...cache, ...patch });
  const saved = await invoke<ChromePrefs>("set_chrome_prefs", { prefs: next });
  cache = normalize(saved);
  notify(cache);
  return getChromePrefs();
}

let eventUnsub: UnlistenFn | null = null;

export async function bindChromePrefsEvents(): Promise<() => void> {
  if (eventUnsub) return eventUnsub;
  eventUnsub = await listen<ChromePrefs>("chrome-prefs", (ev) => {
    cache = normalize(ev.payload);
    hydrated = true;
    notify(cache);
  });
  return () => {
    eventUnsub?.();
    eventUnsub = null;
  };
}

export function chromePrefsHydrated(): boolean {
  return hydrated;
}
