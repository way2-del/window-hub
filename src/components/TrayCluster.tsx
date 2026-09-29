import { useProgressiveGlyphs } from "../features/tray/useProgressiveGlyphs";
import ControlCenterButton from "../features/controlCenter/ControlCenterButton";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hideChromeHoverTip, hostTipPointerProps, installChromeHoverTipGlobalDismiss } from "../chromeHoverTip";
import {
  moveIdInOrder,
  pickDropTarget,
  sameOrder,
} from "../chromeReorder";
import { invokeTrayRightClick, armTrayLeftClick, fireTrayLeftDouble } from "../trayInvoke";
import { clickTrace } from "../clickTrace";
import { isTrayPinnedKey } from "../scenarioGates";
import {
  bindChromePrefsEvents,
  getChromePrefs,
  hydrateChromePrefs,
  setChromePrefs,
  subscribeChromePrefs,
} from "../chromePrefs";
import {
  orderedVisibleChromeChips,
  type ChromeChipId,
} from "../features/chrome/chromeChipOrder";
import {
  computeTrayIconBudget,
  computeTrayRailMaxWidth,
  planTrayIconFold,
  TRAY_ISLAND_SNUG_GAP,
} from "../features/chrome/trayRailFold";
import {
  getLiveIslandWidth,
  subscribeLiveIslandWidth,
} from "../features/chrome/liveIslandGeometry";
import { setTrayRailFold } from "../features/chrome/trayRailFoldBus";
import { requestIslandCollapseIfExpanded } from "../features/chrome/islandCollapseRequest";
import { emit } from "@tauri-apps/api/event";

export type TrayIconInfo = {
  id: string;
  /** Reboot-stable key for 常显 (guid or exe:path:uid). */
  pin_key?: string;
  tooltip: string;
  process: string;
  uid: number;
  hwnd: number;
  callback_msg: number;
  version?: number;
  icon_png_base64: string;
  area: string;
  flashing?: boolean;
  /** Third-party IME notify icons — keep on the rail when present. */
  resident?: boolean;
  /** Windows shell tray (蓝牙/资源管理器等) — never auto-rail on flash. */
  system_tray?: boolean;
};

export type TrayPrefs = {
  pinned: string[];
  menu_heights?: Record<string, number>;
  /** pin_key → flash → island; missing = true */
  flash_notify?: Record<string, boolean>;
};

/** Host-owned Input Indicator (not a Shell_NotifyIcon). */
export type InputLangState = {
  langAbbr: string;
  langName: string;
  imeName: string;
  imeOpen: boolean;
  imeCapable: boolean;
  langId: number;
  hkl?: number;
  profileType?: number;
  clsid?: string;
  guidProfile?: string;
};

/** Host-owned WLAN / Ethernet indicator (Shell network chrome vanishes with taskbar). */
export type WifiState = {
  enabled: boolean;
  connected: boolean;
  ssid: string;
  signal: number;
  ip: string;
  linkMbps: number;
  mac: string;
  secured: boolean;
  /** Wired link up — tray shows Ethernet glyph instead of Wi‑Fi bars. */
  ethernetConnected?: boolean;
  ethernetName?: string;
  ethernetIp?: string;
  ethernetLinkMbps?: number;
  ethernetMac?: string;
};

export type WifiNetwork = {
  ssid: string;
  signal: number;
  secured: boolean;
  connected: boolean;
  hasProfile: boolean;
  profileName?: string;
  auth?: string;
};

const FALLBACK_WIFI: WifiState = {
  enabled: true,
  connected: false,
  ssid: "",
  signal: 0,
  ip: "",
  linkMbps: 0,
  mac: "",
  secured: false,
  ethernetConnected: false,
  ethernetName: "",
  ethernetIp: "",
  ethernetLinkMbps: 0,
  ethernetMac: "",
};

/** Computer + cable — matches Windows Ethernet tray glyph. */
function EthernetGlyph() {
  return (
    <svg className="tray-wifi-glyph" width="15" height="15" viewBox="0 0 24 24" aria-hidden>
      <rect
        x="3"
        y="3"
        width="14"
        height="11"
        rx="1.8"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      />
      <path
        d="M7 20h6M10 14v6"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M17.5 8.5h2.2a1.3 1.3 0 0 1 1.3 1.3v3.4a1.3 1.3 0 0 1-1.3 1.3H17.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19.2 10.2v2.6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Signal bars SVG — thicker 2-arc + tip; off / weak / mid / full. */
function WifiGlyph({ state }: { state: WifiState }) {
  if (state.ethernetConnected) {
    return <EthernetGlyph />;
  }
  const level = !state.enabled
    ? 0
    : !state.connected
      ? 1
      : state.signal >= 55
        ? 3
        : state.signal >= 25
          ? 2
          : 1;
  const tip = level >= 1 ? 1 : 0.22;
  const mid = level >= 2 ? 1 : 0.22;
  const outer = level >= 3 ? 1 : 0.22;
  return (
    <svg className="tray-wifi-glyph" width="15" height="15" viewBox="1 5 22 18" aria-hidden>
      <circle cx="12" cy="19.2" r="2.3" fill="currentColor" opacity={tip} />
      <path
        d="M7.2 13.8a6.9 6.9 0 0 1 9.6 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={mid}
      />
      <path
        d="M3.6 9a12 12 0 0 1 16.8 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={outer}
      />
      {!state.enabled ? (
        <path
          d="M5 5.5 L19 20"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.7"
          strokeLinecap="round"
        />
      ) : null}
    </svg>
  );
}

const FALLBACK_LANG: InputLangState = {
  langAbbr: "中",
  langName: "中文(简体)",
  imeName: "",
  imeOpen: true,
  imeCapable: true,
  langId: 0x0804,
  hkl: 0,
};

/** Stable pin identity — prefer pin_key, fall back to runtime id. */
export function trayPinKey(icon: TrayIconInfo): string {
  const k = (icon.pin_key || "").trim();
  return k || icon.id;
}

/** Lookup keys: pin_key, process stem — reboot-stable (not hwnd:uid). */
export function trayGlyphKeys(icon: TrayIconInfo): string[] {
  const keys: string[] = [];
  const push = (s: string) => {
    const k = s.trim().toLowerCase();
    if (!k || keys.includes(k)) return;
    keys.push(k);
  };
  push(trayPinKey(icon));
  const proc = (icon.process || "").trim().toLowerCase();
  if (proc) {
    push(`proc:${proc}`);
    push(proc);
  }
  const id = (icon.id || "").trim().toLowerCase();
  if (id && !id.includes(":")) push(id);
  return keys;
}

/** Session glyph store keyed by pin_key / process. No localStorage (sync JSON froze UI). */
export function createTrayGlyphCache() {
  const mem = new Map<string, string>();

  const lookup = (icon: TrayIconInfo): string | undefined => {
    for (const k of trayGlyphKeys(icon)) {
      const hit = mem.get(k);
      if (hit) return hit;
    }
    return undefined;
  };

  const remember = (icon: TrayIconInfo, png: string) => {
    if (!png || png.length < 32) return;
    for (const k of trayGlyphKeys(icon)) {
      mem.set(k, png);
    }
    const id = (icon.id || "").trim().toLowerCase();
    if (id) mem.set(id, png);
  };

  const merge = (list: TrayIconInfo[]): TrayIconInfo[] =>
    list.map((i) => {
      if (i.icon_png_base64) {
        remember(i, i.icon_png_base64);
        return i;
      }
      const g = lookup(i);
      return g ? { ...i, icon_png_base64: g } : i;
    });

  const missingIds = (list: TrayIconInfo[]): string[] =>
    list.filter((i) => !i.icon_png_base64 && !lookup(i)).map((i) => i.id);

  const ingest = (idToPng: Record<string, string>, list: TrayIconInfo[]) => {
    const byId = new Map(list.map((i) => [i.id, i]));
    let changed = false;
    for (const [id, png] of Object.entries(idToPng ?? {})) {
      if (!png || png.length < 32) continue;
      const icon = byId.get(id);
      if (icon) remember(icon, png);
      else mem.set(id.toLowerCase(), png);
      changed = true;
    }
    return changed;
  };

  return { merge, missingIds, ingest, remember, lookup };
}

export function isTrayPinned(
  icon: TrayIconInfo,
  pinned: Set<string> | string[],
  liveTrayKeys?: Iterable<string>,
): boolean {
  const key = trayPinKey(icon);
  const live =
    liveTrayKeys ??
    (pinned instanceof Set ? pinned : pinned);
  return isTrayPinnedKey(key, icon.id, pinned, live);
}

/** Third-party IME notify icons that do show up in the tray hook. */
export function isTrayResident(icon: TrayIconInfo): boolean {
  if (icon.resident) return true;
  const tip = (icon.tooltip || "").trim();
  const tipL = tip.toLowerCase();
  const proc = (icon.process || "").trim().toLowerCase();
  const key = (icon.pin_key || icon.id || "").trim().toLowerCase();
  if (
    key === "a59b00b9-f6cd-4fed-a1dc-0f4064a12831" ||
    key === "2c77a81e-41cc-4178-a3a7-5f8a987568e6"
  ) {
    return true;
  }
  if (
    proc === "textinputhost" ||
    proc === "ctfmon" ||
    proc === "tabtip" ||
    proc.includes("sogou") ||
    proc.includes("inputmethod")
  ) {
    return true;
  }
  if (
    /输入法|语言|ime|language|微软拼音|搜狗|中文/.test(tipL) ||
    tipL.includes("chinese")
  ) {
    return true;
  }
  if (/^[\u4e00-\u9fff]$/.test(tip)) return true;
  return /^(en|eng|chs|cht|jp|jpn|kr|kor|中|英|日|韩)$/i.test(tip);
}

const INPUT_LANG_POPUP_W = 240;
const WIFI_POPUP_W = 280;
const TRAY_POPUP_W = 280;
/** 避开顶栏下方 4px 吸色带，防止弹窗像素污染任务栏色带 */
const TRAY_POPUP_GAP = 8;

function pad2(n: number) {
  return n.toString().padStart(2, "0");
}

/** 参考菜单栏：8月4日 周二 14:43 */
function formatMenuClock(d: Date) {
  const week = ["日", "一", "二", "三", "四", "五", "六"][d.getDay()];
  return `${d.getMonth() + 1}月${d.getDate()}日 周${week} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function trayLabel(icon: TrayIconInfo) {
  return icon.tooltip || icon.process || "未知应用";
}

function TrayGlyph({ icon }: { icon: TrayIconInfo }) {
  if (icon.icon_png_base64) {
    return (
      <img
        className="tray-glyph"
        src={`data:image/png;base64,${icon.icon_png_base64}`}
        alt=""
        draggable={false}
      />
    );
  }
  const letter = trayLabel(icon).charAt(0).toUpperCase();
  return <span className="tray-glyph tray-glyph-fallback">{letter}</span>;
}

function imeChipLabel(state: InputLangState): string {
  const name = (state.imeName || "").trim();
  if (/搜狗/.test(name)) return "搜";
  if (/微信/.test(name)) return "P";
  if (/微软|拼音/.test(name)) return "拼";
  if (/日/.test(name) || state.langAbbr === "あ") return "あ";
  if (name) return name.charAt(0);
  // Brand chip only — never mirror 中/英 / never show "0000".
  return "拼";
}

function sanitizeLangAbbr(raw: string | undefined | null): string {
  const s = (raw || "").trim();
  if (!s || s === "0000" || s === "IN" || /^[0-9A-Fa-f]{4}$/.test(s)) return "中";
  // Keep 英 / EN / 中 / 繁 / …
  return s;
}

async function clickTray(icon: TrayIconInfo, action: "left" | "right" | "left-double") {
  try {
    if (action === "right") {
      await invokeTrayRightClick(icon);
    } else if (action === "left-double") {
      fireTrayLeftDouble(icon);
    } else {
      armTrayLeftClick(icon);
    }
  } catch (e) {
    console.error(e);
  }
}

/** Cache strip origin — awaiting outerPosition/scaleFactor on every click can freeze after island resize. */
let cachedAnchor: { factor: number; ox: number; oy: number; at: number } | null = null;

async function popupAnchor(el: HTMLElement, width: number) {
  const now = Date.now();
  if (!cachedAnchor || now - cachedAnchor.at > 800) {
    const win = getCurrentWindow();
    const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    cachedAnchor = { factor, ox: outer.x, oy: outer.y, at: now };
  }
  const { factor, ox, oy } = cachedAnchor;
  const rect = el.getBoundingClientRect();
  const logicalX = ox / factor;
  const logicalY = oy / factor;
  const x = logicalX + rect.right - width;
  const y = logicalY + rect.bottom + TRAY_POPUP_GAP;
  return { x, y };
}

export default function TrayCluster({
  open,
  onOpenChange,
  compactChipsOnly = false,
  onRailWidthChange,
  islandWidth = 300,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Hybrid tier: hide resident tray-icon rail; chevron still follows showTrayMenu. */
  compactChipsOnly?: boolean;
  /** Report rail width (logical px) so right shortcuts can leave a gap. */
  onRailWidthChange?: (width: number) => void;
  /** Island width (logical px); live ResizeObserver can override while morphing. */
  islandWidth?: number;
}) {
  const [chrome, setChrome] = useState(() => getChromePrefs());
  const showTrayIcons = chrome.showTray && !compactChipsOnly;
  /** Dropdown chevron is independent of resident tray icons. */
  const showTrayMenu = chrome.showTrayMenu;
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [menuHeights, setMenuHeights] = useState<Record<string, number>>({});
  const glyphCacheRef = useRef(createTrayGlyphCache());
  const [liveIslandW, setLiveIslandW] = useState(islandWidth);
  const [fixedChromeW, setFixedChromeW] = useState(0);
  const [viewportW, setViewportW] = useState(
    () => (typeof window !== "undefined" ? window.innerWidth : 1280),
  );
  const [now, setNow] = useState(() => new Date());
  const [inputLang, setInputLang] = useState<InputLangState>(FALLBACK_LANG);
  const [wifi, setWifi] = useState<WifiState>(FALLBACK_WIFI);
  const [langMenuOpen, setLangMenuOpen] = useState(false);
  const [wifiMenuOpen, setWifiMenuOpen] = useState(false);
  const [ctrlHeld, setCtrlHeld] = useState(false);
  const [dragKey, setDragKey] = useState<string | null>(null);
  const [dropHint, setDropHint] = useState<{ toId: string; place: "before" | "after" } | null>(
    null,
  );
  const [chipDragId, setChipDragId] = useState<ChromeChipId | null>(null);
  const [chipDropHint, setChipDropHint] = useState<{
    toId: string;
    place: "before" | "after";
  } | null>(null);
  /** After ctrlHeld/chipDragId state — never reference them above. */
  const chipReorderMode = ctrlHeld || chipDragId != null;
  const rootRef = useRef<HTMLDivElement>(null);
  const fixedChromeRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);
  const langChipRef = useRef<HTMLButtonElement>(null);
  const wifiChipRef = useRef<HTMLButtonElement>(null);
  const dragKeyRef = useRef<string | null>(null);
  const dropHintRef = useRef<{ toId: string; place: "before" | "after" } | null>(null);
  const chipDragIdRef = useRef<ChromeChipId | null>(null);
  const chipDropHintRef = useRef<{ toId: string; place: "before" | "after" } | null>(null);
  const pinnedRef = useRef<string[]>([]);
  const menuHeightsRef = useRef<Record<string, number>>({});
  const chromeChipOrderRef = useRef(chrome.chipOrder);
  const suppressClickRef = useRef(false);
  const trayFoldOverflowRef = useRef<string[]>([]);
  dragKeyRef.current = dragKey;
  dropHintRef.current = dropHint;
  chipDragIdRef.current = chipDragId;
  chipDropHintRef.current = chipDropHint;
  pinnedRef.current = pinned;
  menuHeightsRef.current = menuHeights;
  chromeChipOrderRef.current = chrome.chipOrder;

  const visibleChromeChips = useMemo(
    () =>
      orderedVisibleChromeChips(chrome.chipOrder, {
        wifi: chrome.showWifi,
        ime: chrome.showIme,
        controlCenter: chrome.showControlCenter,
        clock: chrome.showClock,
      }),
    [
      chrome.chipOrder,
      chrome.showWifi,
      chrome.showIme,
      chrome.showControlCenter,
      chrome.showClock,
    ],
  );

  useProgressiveGlyphs(
    icons.filter(i => !i.icon_png_base64 && !glyphCacheRef.current.lookup(i)).map(i => i.id),
    map => {
      if (glyphCacheRef.current.ingest(map, icons)) {
        setIcons(prev => glyphCacheRef.current.merge(prev));
      }
    },
  );

  // Subscribe before snapshot. Late snapshots must not replace newer tray events.
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const apply = (list: TrayIconInfo[]) => setIcons(glyphCacheRef.current.merge(list));
    void invoke<TrayPrefs>("get_tray_prefs").then(prefs => {
      if (cancelled) return;
      setPinned(prefs.pinned ?? []);
      setMenuHeights(prefs.menu_heights ?? {});
    }).catch(() => undefined);
    void (async () => {
      try {
        const stop = await listen<TrayIconInfo[]>("tray-icons", event => {
          if (cancelled) return;
          revision++;
          apply(event.payload ?? []);
        });
        if (cancelled) { stop(); return; }
        unlisten = stop;
      } catch { /* the initial snapshot remains available if subscription fails */ }
      const before = revision;
      try {
        const list = await invoke<TrayIconInfo[]>("list_tray_icons");
        if (!cancelled && before === revision) apply(list);
      } catch { /* later events can still supply icons */ }
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);
  useEffect(() => installChromeHoverTipGlobalDismiss(), []);

  useEffect(() => {
    void hydrateChromePrefs().then(setChrome);
    const unsub = subscribeChromePrefs(setChrome);
    let evUnsub: (() => void) | undefined;
    void bindChromePrefsEvents().then((u) => {
      evUnsub = u;
    });
    return () => {
      unsub();
      evUnsub?.();
    };
  }, []);

  // Close tray popup when dropdown is disabled.
  useEffect(() => {
    if (!showTrayMenu && open) {
      onOpenChange(false);
      void invoke("close_tray_popup").catch(() => undefined);
    }
  }, [showTrayMenu, open, onOpenChange]);

  useLayoutEffect(() => {
    setLiveIslandW((prev) => {
      const next = Math.max(islandWidth, getLiveIslandWidth());
      return Math.abs(prev - next) < 1 ? prev : next;
    });
  }, [islandWidth]);

  useEffect(() => {
    return subscribeLiveIslandWidth((w) => {
      setLiveIslandW((prev) => (Math.abs(prev - w) < 0.5 ? prev : w));
    });
  }, []);

  useEffect(() => {
    const syncViewport = () => setViewportW(window.innerWidth);
    syncViewport();
    window.addEventListener("resize", syncViewport);
    return () => window.removeEventListener("resize", syncViewport);
  }, []);

  useEffect(() => {
    const beam = document.querySelector(".island-beam") as HTMLElement | null;
    if (!beam || typeof ResizeObserver === "undefined") return;
    const sync = () => {
      const w = beam.getBoundingClientRect().width;
      if (!Number.isFinite(w) || w < 8) return;
      setLiveIslandW((prev) => (Math.abs(prev - w) < 0.5 ? prev : w));
    };
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(beam);
    return () => ro.disconnect();
  }, []);

  useLayoutEffect(() => {
    const el = fixedChromeRef.current;
    if (!el) {
      setFixedChromeW(0);
      return;
    }
    const report = () => {
      const w = Math.ceil(el.getBoundingClientRect().width);
      setFixedChromeW(Number.isFinite(w) ? w : 0);
    };
    report();
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(report) : null;
    ro?.observe(el);
    return () => ro?.disconnect();
  }, [
    compactChipsOnly,
    chrome.showWifi,
    chrome.showClock,
    chrome.showIme,
    chrome.showControlCenter,
    chrome.showTrayMenu,
    chrome.chipOrder,
    showTrayIcons,
    open,
  ]);

  useEffect(() => {
    if (!onRailWidthChange) return;
    const el = rootRef.current?.querySelector(".tray-rail") as HTMLElement | null;
    if (!el) {
      onRailWidthChange(0);
      return;
    }
    const report = () => {
      const w = Math.ceil(el.getBoundingClientRect().width);
      onRailWidthChange(Number.isFinite(w) ? w : 0);
    };
    report();
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(report) : null;
    ro?.observe(el);
    return () => {
      ro?.disconnect();
      onRailWidthChange(0);
    };
  }, [
    onRailWidthChange,
    compactChipsOnly,
    chrome.showWifi,
    chrome.showClock,
    chrome.showIme,
    chrome.showControlCenter,
    chrome.showTrayMenu,
    chrome.chipOrder,
    showTrayIcons,
  ]);

  useEffect(() => {
    if (!chrome.showClock) return;
    setNow(new Date());
    const id = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(id);
  }, [chrome.showClock]);


  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void (async () => {
      try {
        const lang = await invoke<InputLangState>("get_input_lang");
        if (!cancelled && lang) {
          setInputLang({
            ...lang,
            langAbbr: sanitizeLangAbbr(lang.langAbbr),
          });
        }
      } catch {
        /* keep fallback so chips always paint */
      }

      try {
        const w = await invoke<WifiState>("get_wifi_state");
        if (!cancelled && w) setWifi(w);
      } catch {
        /* keep fallback */
      }

      try {
        unsubs.push(
          await listen<TrayPrefs>("tray-prefs", (ev) => {
            setPinned(ev.payload.pinned ?? []);
            setMenuHeights(ev.payload.menu_heights ?? {});
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<InputLangState>("input-lang", (ev) => {
            if (!ev.payload) return;
            setInputLang({
              ...ev.payload,
              langAbbr: sanitizeLangAbbr(ev.payload.langAbbr),
            });
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<WifiState>("wifi-state", (ev) => {
            if (ev.payload) setWifi(ev.payload);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-opened", () => {
            onOpenChange(true);
            void emit("tray-rail-fold", {
              overflowIds: trayFoldOverflowRef.current,
            }).catch(() => undefined);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-closed", () => {
            onOpenChange(false);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("input-lang-popup-opened", () => {
            setLangMenuOpen(true);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("input-lang-popup-closed", () => {
            setLangMenuOpen(false);
            void invoke<InputLangState>("get_input_lang")
              .then((lang) => {
                if (!lang) return;
                setInputLang({
                  ...lang,
                  langAbbr: sanitizeLangAbbr(lang.langAbbr),
                });
              })
              .catch(() => undefined);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("wifi-popup-opened", () => {
            setWifiMenuOpen(true);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("wifi-popup-closed", () => {
            setWifiMenuOpen(false);
            void invoke<WifiState>("get_wifi_state")
              .then((w) => {
                if (w) setWifi(w);
              })
              .catch(() => undefined);
          }),
        );
      } catch {
        /* noop */
      }
    })();

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, [onOpenChange]);

  // 父级要求关闭时（展开灵动岛等）同步关掉独立弹窗
  useEffect(() => {
    if (open) return;
    void invoke("is_tray_popup_open")
      .then((visible) => {
        if (visible) return invoke("close_tray_popup");
      })
      .catch(() => undefined);
  }, [open]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Control") {
        setCtrlHeld(true);
        void hideChromeHoverTip();
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === "Control") {
        setCtrlHeld(false);
        // Do NOT cancel an in-flight drag on Ctrl release — finish on pointerup.
      }
    };
    const onBlur = () => {
      setCtrlHeld(false);
      if (dragKeyRef.current) {
        setDragKey(null);
        setDropHint(null);
        dropHintRef.current = null;
        dragKeyRef.current = null;
      }
      if (chipDragIdRef.current) {
        setChipDragId(null);
        setChipDropHint(null);
        chipDropHintRef.current = null;
        chipDragIdRef.current = null;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const liveTrayKeys = useMemo(
    () => icons.map((i) => trayPinKey(i)).filter(Boolean),
    [icons],
  );
  const pinnedIcons = useMemo(() => {
    const pinnedOnly = icons.filter((i) => isTrayPinned(i, pinnedSet, liveTrayKeys));
    const rank = new Map(pinned.map((k, i) => [k, i]));
    return [...pinnedOnly].sort((a, b) => {
      const ra = rank.get(trayPinKey(a)) ?? rank.get(a.id) ?? 1e9;
      const rb = rank.get(trayPinKey(b)) ?? rank.get(b.id) ?? 1e9;
      return ra - rb;
    });
  }, [icons, pinnedSet, pinned, liveTrayKeys]);
  const railIcons = useMemo(() => {
    const seen = new Set<string>();
    const out: TrayIconInfo[] = [];
    const push = (icon: TrayIconInfo) => {
      if (seen.has(icon.id)) return;
      seen.add(icon.id);
      out.push(icon);
    };
    const resident = icons.filter((i) => isTrayResident(i));
    const residentIds = new Set(resident.map((i) => i.id));
    // User-ordered pinned apps first (Ctrl+drag persists via pinned[] order).
    for (const icon of pinnedIcons) {
      if (!residentIds.has(icon.id)) push(icon);
    }
    for (const icon of resident) push(icon);
    // 第三方应用闪动时临时上图栏；系统托盘（蓝牙/资源管理器等）不自动出现
    for (const icon of icons) {
      if (icon.flashing && !icon.system_tray) push(icon);
    }
    return out;
  }, [icons, pinnedIcons]);

  const trayFold = useMemo(() => {
    if (!showTrayIcons || railIcons.length === 0) {
      return { visibleIds: [] as string[], overflowIds: [] as string[] };
    }
    const iw = Math.max(islandWidth, liveIslandW, getLiveIslandWidth());
    const maxRail = computeTrayRailMaxWidth(
      viewportW,
      iw,
      TRAY_ISLAND_SNUG_GAP,
    );
    const budget = computeTrayIconBudget(maxRail, fixedChromeW);
    return planTrayIconFold(
      railIcons.map((i) => i.id),
      budget,
    );
  }, [
    showTrayIcons,
    railIcons,
    islandWidth,
    liveIslandW,
    viewportW,
    fixedChromeW,
  ]);

  trayFoldOverflowRef.current = trayFold.overflowIds;
  const stashedIdSet = useMemo(
    () => new Set(trayFold.overflowIds),
    [trayFold.overflowIds],
  );

  useEffect(() => {
    setTrayRailFold({ overflowIds: trayFold.overflowIds });
    void emit("tray-rail-fold", { overflowIds: trayFold.overflowIds }).catch(
      () => undefined,
    );
  }, [trayFold.overflowIds]);

  const persistPinnedOrder = useCallback(async (nextPinned: string[]) => {
    setPinned(nextPinned);
    try {
      const prefs = await invoke<TrayPrefs>("set_tray_prefs", {
        pinned: nextPinned,
        menuHeights: menuHeightsRef.current,
      });
      setPinned(prefs.pinned ?? nextPinned);
      setMenuHeights(prefs.menu_heights ?? menuHeightsRef.current);
    } catch (err) {
      console.error("[TrayCluster] persist order", err);
    }
  }, []);

  const persistChipOrder = useCallback(async (nextOrder: ChromeChipId[]) => {
    setChrome((prev) => ({ ...prev, chipOrder: nextOrder }));
    try {
      const saved = await setChromePrefs({ chipOrder: nextOrder });
      setChrome(saved);
    } catch (err) {
      console.error("[TrayCluster] persist chip order", err);
    }
  }, []);

  const onTrayReorderDown = useCallback(
    (icon: TrayIconInfo, e: ReactPointerEvent<HTMLButtonElement>) => {
      if (!e.ctrlKey || e.button !== 0 || isTrayResident(icon)) return;
      e.preventDefault();
      e.stopPropagation();
      void hideChromeHoverTip();
      const key = trayPinKey(icon);
      setDragKey(key);
      setDropHint(null);
      dropHintRef.current = null;
      dragKeyRef.current = key;
      suppressClickRef.current = false;

      const onMove = (ev: PointerEvent) => {
        if (!dragKeyRef.current) return;
        suppressClickRef.current = true;
        const rail = rootRef.current?.querySelector(".tray-rail");
        if (!rail) return;
        const units = Array.from(rail.querySelectorAll<HTMLElement>("[data-tray-pin]"))
          .map((el) => {
            const r = el.getBoundingClientRect();
            return { id: el.dataset.trayPin || "", left: r.left, width: r.width };
          })
          .filter((u) => u.id);
        const hint = pickDropTarget(ev.clientX, units, dragKeyRef.current);
        dropHintRef.current = hint;
        setDropHint(hint);
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        const fromKey = dragKeyRef.current;
        const hint = dropHintRef.current;
        setDragKey(null);
        setDropHint(null);
        dropHintRef.current = null;
        dragKeyRef.current = null;
        if (!fromKey || !hint) return;
        const next = moveIdInOrder(pinnedRef.current, fromKey, hint.toId, hint.place);
        if (sameOrder(pinnedRef.current, next)) return;
        void persistPinnedOrder(next);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    [persistPinnedOrder],
  );

  const onChromeChipReorderDown = useCallback(
    (chipId: ChromeChipId, e: ReactPointerEvent<HTMLElement>) => {
      if (!e.ctrlKey || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      void hideChromeHoverTip();
      setChipDragId(chipId);
      setChipDropHint(null);
      chipDropHintRef.current = null;
      chipDragIdRef.current = chipId;
      suppressClickRef.current = false;

      const onMove = (ev: PointerEvent) => {
        if (!chipDragIdRef.current) return;
        suppressClickRef.current = true;
        const rail = fixedChromeRef.current;
        if (!rail) return;
        const units = Array.from(
          rail.querySelectorAll<HTMLElement>("[data-chrome-chip]"),
        )
          .map((el) => {
            const r = el.getBoundingClientRect();
            return {
              id: el.dataset.chromeChip || "",
              left: r.left,
              width: r.width,
            };
          })
          .filter((u) => u.id);
        const hint = pickDropTarget(ev.clientX, units, chipDragIdRef.current);
        chipDropHintRef.current = hint;
        setChipDropHint(hint);
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        const fromId = chipDragIdRef.current;
        const hint = chipDropHintRef.current;
        setChipDragId(null);
        setChipDropHint(null);
        chipDropHintRef.current = null;
        chipDragIdRef.current = null;
        if (!fromId || !hint) return;
        const next = moveIdInOrder(
          chromeChipOrderRef.current,
          fromId,
          hint.toId,
          hint.place,
        ).filter((id): id is ChromeChipId =>
          ["wifi", "ime", "controlCenter", "clock"].includes(id),
        );
        if (sameOrder(chromeChipOrderRef.current, next)) return;
        void persistChipOrder(next);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    [persistChipOrder],
  );

  async function openLangMenu() {
    clickTrace("fe-tray", "openLangMenu");
    try {
      clickTrace("fe-tray", "before is_input_lang_popup_open");
      const visible = await invoke<boolean>("is_input_lang_popup_open");
      clickTrace("fe-tray", `lang is_open=${visible}`);
      if (visible || langMenuOpen) {
        await invoke("close_input_lang_popup");
        setLangMenuOpen(false);
        return;
      }
      const el = langChipRef.current ?? chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, INPUT_LANG_POPUP_W);
      clickTrace("fe-tray", "before open_input_lang_popup");
      await invoke("open_input_lang_popup", { x, y });
      clickTrace("fe-tray", "after open_input_lang_popup");
      setLangMenuOpen(true);
    } catch (e) {
      clickTrace("fe-tray", `openLangMenu error ${String(e)}`);
      console.error(e);
    }
  }

  async function openWifiMenu() {
    clickTrace("fe-tray", "openWifiMenu");
    try {
      clickTrace("fe-tray", "before is_wifi_popup_open");
      const visible = await invoke<boolean>("is_wifi_popup_open");
      clickTrace("fe-tray", `wifi is_open=${visible}`);
      if (visible || wifiMenuOpen) {
        await invoke("close_wifi_popup");
        setWifiMenuOpen(false);
        return;
      }
      const el = wifiChipRef.current ?? chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, WIFI_POPUP_W);
      clickTrace("fe-tray", "before open_wifi_popup");
      await invoke("open_wifi_popup", { x, y });
      clickTrace("fe-tray", "after open_wifi_popup");
      setWifiMenuOpen(true);
    } catch (e) {
      clickTrace("fe-tray", `openWifiMenu error ${String(e)}`);
      console.error(e);
    }
  }

  async function onLangClick() {
    // Always toggle 中↔英/EN; menu is on the IME chip / right-click.
    try {
      const next = await invoke<InputLangState>("toggle_input_ime");
      if (next) {
        setInputLang({
          ...next,
          langAbbr: sanitizeLangAbbr(next.langAbbr),
        });
      }
    } catch (e) {
      console.error(e);
      await openLangMenu();
    }
  }

  async function onLangContext(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    await openLangMenu();
  }

  const pressedPopupOpen = useRef<boolean | null>(null);
  async function togglePopup() {
    const beforePress = pressedPopupOpen.current;
    pressedPopupOpen.current = null;
    clickTrace("fe-tray", "togglePopup click");
    void hideChromeHoverTip();
    try {
      if (await requestIslandCollapseIfExpanded()) {
        clickTrace("fe-tray", "island expanded → collapse instead of tray popup");
        if (beforePress ?? open) {
          await invoke("close_tray_popup").catch(() => undefined);
          onOpenChange(false);
        }
        return;
      }
      if (beforePress ?? open) {
        clickTrace("fe-tray", "before close_tray_popup (local open)");
        await invoke("close_tray_popup");
        clickTrace("fe-tray", "after close");
        onOpenChange(false);
        return;
      }
      clickTrace("fe-tray", "before is_tray_popup_open");
      const visible = await invoke<boolean>("is_tray_popup_open");
      clickTrace("fe-tray", `is_open=${visible} open=${open}`);
      if (visible) {
        clickTrace("fe-tray", "before close_tray_popup");
        await invoke("close_tray_popup");
        clickTrace("fe-tray", "after close");
        onOpenChange(false);
        return;
      }
      const el = chevronRef.current;
      if (!el) {
        clickTrace("fe-tray", "no chevron el");
        return;
      }
      clickTrace("fe-tray", "before popupAnchor");
      const { x, y } = await popupAnchor(el, TRAY_POPUP_W);
      clickTrace("fe-tray", `anchor x=${x.toFixed(0)} y=${y.toFixed(0)}`);
      clickTrace("fe-tray", "before open_tray_popup");
      await invoke("open_tray_popup", { x, y });
      clickTrace("fe-tray", "after open_tray_popup");
      onOpenChange(true);
    } catch (e) {
      clickTrace("fe-tray", `error ${String(e)}`);
      console.error(e);
    }
  }

  const langTip = [inputLang.langName, inputLang.imeName, "单击切换中/英 · 右键打开输入法菜单"]
    .filter(Boolean)
    .join("\n");

  const imeTip = [inputLang.imeName || "输入法", "单击打开输入法菜单"].filter(Boolean).join("\n");

  const wifiTip = wifi.ethernetConnected
    ? `${wifi.ethernetName || "以太网"}${
        wifi.ethernetIp ? ` · ${wifi.ethernetIp}` : ""
      }${wifi.ethernetLinkMbps ? ` · ${wifi.ethernetLinkMbps} Mbps` : ""}\n单击打开网络菜单`
    : !wifi.enabled
      ? "Wi‑Fi 已关闭\n单击打开 WLAN 菜单"
      : wifi.connected && wifi.ssid
        ? `${wifi.ssid}${wifi.signal ? ` · ${wifi.signal}%` : ""}\n单击打开 WLAN 菜单`
        : "未连接\n单击打开 WLAN 菜单";

  const wifiOn = Boolean(
    wifi.ethernetConnected || (wifi.enabled && wifi.connected),
  );
  const wifiAria = wifi.ethernetConnected
    ? `有线网络 ${wifi.ethernetName || "已连接"}`
    : wifi.enabled
      ? wifi.connected
        ? `Wi‑Fi ${wifi.ssid || "已连接"}`
        : "Wi‑Fi 未连接"
      : "Wi‑Fi 已关闭";

  return (
    <div
      className={`tray-cluster${ctrlHeld || dragKey ? " is-reorder" : ""}${
        dragKey ? " is-dragging" : ""
      }`}
      ref={rootRef}
      onClick={(e) => e.stopPropagation()}
    >
      <div className={`tray-rail${compactChipsOnly ? " is-compact-chips" : ""}`}>
        {showTrayIcons ? (
          <div className="tray-icons" aria-hidden={false}>
            {railIcons.map((icon) => {
              const pinKey = trayPinKey(icon);
              const stashed = stashedIdSet.has(icon.id);
              const canReorder =
                !stashed &&
                !isTrayResident(icon) &&
                isTrayPinned(icon, pinnedSet, liveTrayKeys);
              return (
                <button
                  key={icon.id}
                  type="button"
                  data-tray-pin={canReorder ? pinKey : undefined}
                  className={`tray-icon-btn${stashed ? " is-stashed" : ""}${
                    icon.flashing && !stashed ? " is-flashing" : ""
                  }${dragKey === pinKey ? " is-dragging" : ""}${
                    dropHint?.toId === pinKey ? ` is-drop-${dropHint.place}` : ""
                  }`}
                  aria-hidden={stashed}
                  tabIndex={stashed ? -1 : undefined}
                  {...(stashed || dragKey || ctrlHeld
                    ? {}
                    : hostTipPointerProps(trayLabel(icon)))}
                  onPointerDown={(e) => {
                    if (canReorder) onTrayReorderDown(icon, e);
                  }}
                  onClick={() => {
                    if (stashed) return;
                    if (suppressClickRef.current || ctrlHeld || dragKey) {
                      suppressClickRef.current = false;
                      return;
                    }
                    void hideChromeHoverTip();
                    void clickTray(icon, "left");
                  }}
                  onDoubleClick={(e) => {
                    e.preventDefault();
                    if (stashed || suppressClickRef.current || ctrlHeld || dragKey)
                      return;
                    void hideChromeHoverTip();
                    void clickTray(icon, "left-double");
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    if (stashed || ctrlHeld || dragKey) return;
                    void clickTray(icon, "right");
                  }}
                >
                  <TrayGlyph icon={icon} />
                </button>
              );
            })}
          </div>
        ) : null}

        <div
          className={`tray-fixed-chrome${chipDragId ? " is-chip-dragging" : ""}${
            chipReorderMode ? " is-chip-reorder" : ""
          }`}
          ref={fixedChromeRef}
        >
          {visibleChromeChips.map((chipId) => {
            const unitClass = [
              "tray-chrome-chip",
              chipDragId === chipId ? "is-dragging" : "",
              chipDropHint?.toId === chipId ? `is-drop-${chipDropHint.place}` : "",
            ]
              .filter(Boolean)
              .join(" ");
            const chipPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
              if (e.ctrlKey) onChromeChipReorderDown(chipId, e);
            };
            const reorderHit = chipReorderMode ? (
              <div
                className="tray-chrome-reorder-hit"
                aria-hidden
                onPointerDown={chipPointerDown}
              />
            ) : null;

            if (chipId === "wifi") {
              return (
                <div
                  key="wifi"
                  className={unitClass}
                  data-chrome-chip="wifi"
                  onPointerDown={chipPointerDown}
                >
                  {reorderHit}
                  <button
                    ref={wifiChipRef}
                    type="button"
                    className={`tray-wifi-btn${wifiMenuOpen ? " is-open" : ""}${
                      wifiOn ? " is-on" : ""
                    }${!wifi.enabled && !wifi.ethernetConnected ? " is-off" : ""}`}
                    {...(chipReorderMode ? {} : hostTipPointerProps(wifiTip))}
                    aria-label={wifiAria}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => {
                      if (suppressClickRef.current || chipReorderMode) {
                        suppressClickRef.current = false;
                        return;
                      }
                      clickTrace("fe-tray", "wifi click");
                      void hideChromeHoverTip();
                      void openWifiMenu();
                    }}
                  >
                    <WifiGlyph state={wifi} />
                  </button>
                </div>
              );
            }

            if (chipId === "ime") {
              return (
                <div
                  key="ime"
                  className={`${unitClass} is-ime-pair`}
                  data-chrome-chip="ime"
                  onPointerDown={chipPointerDown}
                >
                  {reorderHit}
                  <button
                    ref={langChipRef}
                    type="button"
                    className={`tray-lang-btn${langMenuOpen ? " is-open" : ""}`}
                    {...(chipReorderMode ? {} : hostTipPointerProps(langTip))}
                    aria-label={`输入语言 ${inputLang.langAbbr}`}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => {
                      if (suppressClickRef.current || chipReorderMode) {
                        suppressClickRef.current = false;
                        return;
                      }
                      clickTrace("fe-tray", "lang click");
                      void hideChromeHoverTip();
                      void onLangClick();
                    }}
                    onContextMenu={(e) => {
                      if (chipReorderMode) return;
                      void onLangContext(e);
                    }}
                  >
                    <span className="tray-lang-abbr">
                      {sanitizeLangAbbr(inputLang.langAbbr)}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`tray-ime-btn${
                      inputLang.langAbbr === "中" || inputLang.imeOpen
                        ? " is-open"
                        : ""
                    }${langMenuOpen ? " is-menu" : ""}`}
                    {...(chipReorderMode ? {} : hostTipPointerProps(imeTip))}
                    aria-label={`输入法 ${inputLang.imeName || "IME"}`}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => {
                      if (suppressClickRef.current || chipReorderMode) {
                        suppressClickRef.current = false;
                        return;
                      }
                      clickTrace("fe-tray", "ime click");
                      void hideChromeHoverTip();
                      void openLangMenu();
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      if (chipReorderMode) return;
                      void openLangMenu();
                    }}
                  >
                    <span className="tray-ime-mark">{imeChipLabel(inputLang)}</span>
                  </button>
                </div>
              );
            }

            if (chipId === "controlCenter") {
              return (
                <div
                  key="controlCenter"
                  className={unitClass}
                  data-chrome-chip="controlCenter"
                  onPointerDown={chipPointerDown}
                >
                  {reorderHit}
                  <ControlCenterButton reorderLocked={chipReorderMode} />
                </div>
              );
            }

            return (
              <div
                key="clock"
                className={unitClass}
                data-chrome-chip="clock"
                onPointerDown={chipPointerDown}
              >
                {reorderHit}
                <button
                  type="button"
                  className="tray-clock"
                  {...(chipReorderMode
                    ? {}
                    : hostTipPointerProps("打开通知中心"))}
                  aria-label="打开通知中心"
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => {
                    if (suppressClickRef.current || chipReorderMode) {
                      suppressClickRef.current = false;
                      return;
                    }
                    clickTrace("fe-tray", "clock click");
                    void hideChromeHoverTip();
                    void invoke("open_notification_center").catch((e) =>
                      console.error(e),
                    );
                  }}
                >
                  <time dateTime={now.toISOString()}>{formatMenuClock(now)}</time>
                </button>
              </div>
            );
          })}

          {showTrayMenu ? (
            <button
              key={`tray-chevron-${trayFold.overflowIds.length}`}
              ref={chevronRef}
              type="button"
              className={`tray-chevron${open ? " is-open" : ""}${
                trayFold.overflowIds.length > 0 ? " has-overflow" : ""
              }`}
              aria-label={open ? "收起托盘" : "展开托盘"}
              aria-expanded={open}
              onPointerDown={() => {
                pressedPopupOpen.current = open;
              }}
              onPointerCancel={() => {
                pressedPopupOpen.current = null;
              }}
              onMouseDown={(e) => {
                e.preventDefault();
              }}
              onClick={() => void togglePopup()}
            >
              <svg width="9" height="9" viewBox="0 0 12 12" aria-hidden>
                <path
                  d="M2.2 7.8 L6 4 L9.8 7.8"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
