import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hideChromeHoverTip, hostTipPointerProps } from "../chromeHoverTip";
import {
  moveIdInOrder,
  pickDropTarget,
  sameOrder,
} from "../chromeReorder";

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
};

export type TrayPrefs = {
  pinned: string[];
  menu_heights?: Record<string, number>;
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

/** Host-owned WLAN indicator (Shell WLAN chrome vanishes with taskbar). */
export type WifiState = {
  enabled: boolean;
  connected: boolean;
  ssid: string;
  signal: number;
  ip: string;
  linkMbps: number;
  mac: string;
  secured: boolean;
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
};

/** Signal bars SVG — thicker 2-arc + tip; off / weak / mid / full. */
function WifiGlyph({ state }: { state: WifiState }) {
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

export function isTrayPinned(icon: TrayIconInfo, pinned: Set<string> | string[]): boolean {
  const set = pinned instanceof Set ? pinned : new Set(pinned);
  return set.has(trayPinKey(icon)) || set.has(icon.id);
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

async function clickTray(icon: TrayIconInfo, action: "left" | "right") {
  try {
    await invoke("invoke_tray_icon", {
      id: icon.id,
      hwnd: icon.hwnd,
      callbackMsg: icon.callback_msg,
      uid: icon.uid,
      version: icon.version ?? 0,
      action,
    });
  } catch (e) {
    console.error(e);
  }
}

async function popupAnchor(el: HTMLElement, width: number) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  const logicalX = outer.x / factor;
  const logicalY = outer.y / factor;
  const x = logicalX + rect.right - width;
  const y = logicalY + rect.bottom + TRAY_POPUP_GAP;
  return { x, y };
}

const INPUT_LANG_POPUP_W = 240;
const WIFI_POPUP_W = 280;

export default function TrayCluster({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [menuHeights, setMenuHeights] = useState<Record<string, number>>({});
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
  const rootRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);
  const langChipRef = useRef<HTMLButtonElement>(null);
  const wifiChipRef = useRef<HTMLButtonElement>(null);
  const dragKeyRef = useRef<string | null>(null);
  const dropHintRef = useRef<{ toId: string; place: "before" | "after" } | null>(null);
  const pinnedRef = useRef<string[]>([]);
  const menuHeightsRef = useRef<Record<string, number>>({});
  const suppressClickRef = useRef(false);
  dragKeyRef.current = dragKey;
  dropHintRef.current = dropHint;
  pinnedRef.current = pinned;
  menuHeightsRef.current = menuHeights;

  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(t);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void (async () => {
      try {
        const [list, prefs] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
        ]);
        if (!cancelled) {
          setIcons(list);
          setPinned(prefs.pinned ?? []);
          setMenuHeights(prefs.menu_heights ?? {});
        }
      } catch {
        /* noop */
      }

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
          await listen<TrayIconInfo[]>("tray-icons", (ev) => {
            setIcons(ev.payload);
          }),
        );
      } catch {
        /* noop */
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
  const pinnedIcons = useMemo(() => {
    const pinnedOnly = icons.filter((i) => isTrayPinned(i, pinnedSet));
    const rank = new Map(pinned.map((k, i) => [k, i]));
    return [...pinnedOnly].sort((a, b) => {
      const ra = rank.get(trayPinKey(a)) ?? rank.get(a.id) ?? 1e9;
      const rb = rank.get(trayPinKey(b)) ?? rank.get(b.id) ?? 1e9;
      return ra - rb;
    });
  }, [icons, pinnedSet, pinned]);
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
    for (const icon of icons) {
      if (icon.flashing) push(icon);
    }
    return out;
  }, [icons, pinnedIcons]);

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

  async function openLangMenu() {
    try {
      const visible = await invoke<boolean>("is_input_lang_popup_open");
      if (visible || langMenuOpen) {
        await invoke("close_input_lang_popup");
        setLangMenuOpen(false);
        return;
      }
      const el = langChipRef.current ?? chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, INPUT_LANG_POPUP_W);
      await invoke("open_input_lang_popup", { x, y });
      setLangMenuOpen(true);
    } catch (e) {
      console.error(e);
    }
  }

  async function openWifiMenu() {
    try {
      const visible = await invoke<boolean>("is_wifi_popup_open");
      if (visible || wifiMenuOpen) {
        await invoke("close_wifi_popup");
        setWifiMenuOpen(false);
        return;
      }
      const el = wifiChipRef.current ?? chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, WIFI_POPUP_W);
      await invoke("open_wifi_popup", { x, y });
      setWifiMenuOpen(true);
    } catch (e) {
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

  async function togglePopup() {
    try {
      const visible = await invoke<boolean>("is_tray_popup_open");
      if (visible || open) {
        await invoke("close_tray_popup");
        onOpenChange(false);
        return;
      }
      const el = chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, TRAY_POPUP_W);
      await invoke("open_tray_popup", { x, y });
      onOpenChange(true);
    } catch (e) {
      console.error(e);
    }
  }

  const langTip = [inputLang.langName, inputLang.imeName, "单击切换中/英 · 右键打开输入法菜单"]
    .filter(Boolean)
    .join("\n");

  const imeTip = [inputLang.imeName || "输入法", "单击打开输入法菜单"].filter(Boolean).join("\n");

  const wifiTip = !wifi.enabled
    ? "Wi‑Fi 已关闭\n单击打开 WLAN 菜单"
    : wifi.connected && wifi.ssid
      ? `${wifi.ssid}${wifi.signal ? ` · ${wifi.signal}%` : ""}\n单击打开 WLAN 菜单`
      : "未连接\n单击打开 WLAN 菜单";

  return (
    <div
      className={`tray-cluster${ctrlHeld || dragKey ? " is-reorder" : ""}${
        dragKey ? " is-dragging" : ""
      }`}
      ref={rootRef}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="tray-rail">
        {railIcons.map((icon) => {
          const pinKey = trayPinKey(icon);
          const canReorder = !isTrayResident(icon) && isTrayPinned(icon, pinnedSet);
          return (
            <button
              key={icon.id}
              type="button"
              data-tray-pin={canReorder ? pinKey : undefined}
              className={`tray-icon-btn${icon.flashing ? " is-flashing" : ""}${
                dragKey === pinKey ? " is-dragging" : ""
              }${
                dropHint?.toId === pinKey ? ` is-drop-${dropHint.place}` : ""
              }`}
              {...(dragKey || ctrlHeld ? {} : hostTipPointerProps(trayLabel(icon)))}
              onPointerDown={(e) => {
                if (canReorder) onTrayReorderDown(icon, e);
              }}
              onClick={() => {
                if (suppressClickRef.current || ctrlHeld || dragKey) {
                  suppressClickRef.current = false;
                  return;
                }
                void hideChromeHoverTip();
                void clickTray(icon, "left");
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                if (ctrlHeld || dragKey) return;
                void clickTray(icon, "right");
              }}
            >
              <TrayGlyph icon={icon} />
            </button>
          );
        })}

        <button
          ref={wifiChipRef}
          type="button"
          className={`tray-wifi-btn${wifiMenuOpen ? " is-open" : ""}${
            wifi.enabled && wifi.connected ? " is-on" : ""
          }${!wifi.enabled ? " is-off" : ""}`}
          {...hostTipPointerProps(wifiTip)}
          aria-label={
            wifi.enabled
              ? wifi.connected
                ? `Wi‑Fi ${wifi.ssid || "已连接"}`
                : "Wi‑Fi 未连接"
              : "Wi‑Fi 已关闭"
          }
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => { void hideChromeHoverTip(); void openWifiMenu(); }}
        >
          <WifiGlyph state={wifi} />
        </button>

        <button
          ref={langChipRef}
          type="button"
          className={`tray-lang-btn${langMenuOpen ? " is-open" : ""}`}
          {...hostTipPointerProps(langTip)}
          aria-label={`输入语言 ${inputLang.langAbbr}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => { void hideChromeHoverTip(); void onLangClick(); }}
          onContextMenu={(e) => void onLangContext(e)}
        >
          <span className="tray-lang-abbr">{sanitizeLangAbbr(inputLang.langAbbr)}</span>
        </button>
        <button
          type="button"
          className={`tray-ime-btn${
            inputLang.langAbbr === "中" || inputLang.imeOpen ? " is-open" : ""
          }${langMenuOpen ? " is-menu" : ""}`}
          {...hostTipPointerProps(imeTip)}
          aria-label={`输入法 ${inputLang.imeName || "IME"}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => { void hideChromeHoverTip(); void openLangMenu(); }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            void openLangMenu();
          }}
        >
          <span className="tray-ime-mark">{imeChipLabel(inputLang)}</span>
        </button>

        <button
          type="button"
          className="tray-clock"
          {...hostTipPointerProps("打开通知中心")}
          onClick={() => {
            void hideChromeHoverTip();
            void invoke("open_notification_center").catch((e) => console.error(e));
          }}
        >
          <time dateTime={now.toISOString()}>{formatMenuClock(now)}</time>
        </button>

        <button
          ref={chevronRef}
          type="button"
          className={`tray-chevron${open ? " is-open" : ""}`}
          aria-label={open ? "收起托盘" : "展开托盘"}
          aria-expanded={open}
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
      </div>
    </div>
  );
}
