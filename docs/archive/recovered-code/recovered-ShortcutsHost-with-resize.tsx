import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  SHORTCUTS_HEIGHT,
  computeShortcutsBounds,
  type ShortcutsBounds,
} from "../plugins/shortcutsGeometry";
import { pluginRegistry } from "../plugins/registry";
import type { ShortcutsPluginRuntime } from "../plugins/types";
import "./ShortcutsHost.css";

const POPUP_GAP = 8;
const HOVER_OPEN_MS = 140;
const PIN_MIN_W = 56;
const PIN_MAX_W = 220;

export type HostShortcutPin = {
  pluginId: string;
  id: string;
  label: string;
  badge?: string | number | null;
  width: number;
  action: string;
  windowId?: string | null;
  preferGroupId?: string | null;
};

type Props = {
  settingsRef: RefObject<HTMLElement | null>;
  islandWidth: number;
};

function truncate(s: string, n: number) {
  const t = s.trim();
  if (t.length <= n) return t;
  return `${t.slice(0, n - 1)}…`;
}

/** Estimate chip width from label (12px semibold): CJK≈12, ASCII≈7.2, + padding/dot/badge. */
function pinWidthForLabel(
  label: string,
  opts: { activeDot?: boolean; badge?: boolean } = {},
): number {
  let text = 0;
  for (const c of label.trim()) {
    const cp = c.codePointAt(0) ?? 0;
    text += cp > 0xff ? 12 : 7.2;
  }
  let w = 10 + text; // horizontal padding
  if (opts.activeDot) w += 11;
  if (opts.badge) w += 18;
  return Math.max(40, Math.min(PIN_MAX_W, Math.ceil(w)));
}

function WindowsIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  );
}

function PluginIcon({ icon }: { icon?: string }) {
  if (icon === "windows") return <WindowsIcon />;
  return <span className="shortcuts-chip-icon" aria-hidden>◆</span>;
}

async function popupAnchor(el: HTMLElement) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  return {
    x: Math.max(8, outer.x / factor + rect.left),
    y: outer.y / factor + rect.bottom + POPUP_GAP,
  };
}

function normalizePin(raw: Record<string, unknown>): HostShortcutPin {
  const badge = raw.badge;
  return {
    pluginId: String(raw.pluginId ?? ""),
    id: String(raw.id ?? ""),
    label: String(raw.label ?? ""),
    badge:
      typeof badge === "number" || typeof badge === "string"
        ? badge
        : badge == null
          ? null
          : String(badge),
    width: Math.max(PIN_MIN_W, Math.min(PIN_MAX_W, Number(raw.width) || 96)),
    action: String(raw.action ?? "popup.open"),
    windowId: raw.windowId != null ? String(raw.windowId) : null,
    preferGroupId: raw.preferGroupId != null ? String(raw.preferGroupId) : null,
  };
}

export default function ShortcutsHost({ settingsRef, islandWidth }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const anchorRefs = useRef<Map<string, HTMLElement>>(new Map());
  const openingRef = useRef(false);
  const popupOpenRef = useRef(false);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [bounds, setBounds] = useState<ShortcutsBounds>({
    x: 0,
    width: 0,
    height: SHORTCUTS_HEIGHT,
    maxExpandWidth: 0,
  });
  const [popupOpen, setPopupOpen] = useState(false);
  const [popupPluginId, setPopupPluginId] = useState<string | null>(null);
  const [, setRegistryVersion] = useState(0);
  const [expandedPluginId, setExpandedPluginId] = useState<string | null>(null);
  const [pins, setPins] = useState<HostShortcutPin[]>([]);
  const [activeWindowId, setActiveWindowId] = useState<string | null>(null);
  const lastFgId = useRef<string | null>(null);

  const clearHoverTimer = () => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  };

  const recomputeBounds = useCallback(() => {
    const settingsEl = settingsRef.current;
    const shell = hostRef.current?.offsetParent as HTMLElement | null;
    if (!settingsEl || !shell) return;
    const shellRect = shell.getBoundingClientRect();
    const settingsRect = settingsEl.getBoundingClientRect();
    const settingsRight = settingsRect.right - shellRect.left;
    const islandLeft = shellRect.width / 2 - islandWidth / 2;
    setBounds(computeShortcutsBounds(settingsRight, islandLeft));
  }, [settingsRef, islandWidth]);

  useLayoutEffect(() => {
    recomputeBounds();
  }, [recomputeBounds, pins.length, popupOpen]);

  useEffect(() => {
    const onResize = () => recomputeBounds();
    window.addEventListener("resize", onResize);
    const ro =
      settingsRef.current && typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(onResize)
        : null;
    if (settingsRef.current && ro) ro.observe(settingsRef.current);
    return () => {
      window.removeEventListener("resize", onResize);
      ro?.disconnect();
    };
  }, [recomputeBounds, settingsRef]);

  useEffect(() => pluginRegistry.subscribe(() => setRegistryVersion((n) => n + 1)), []);

  // Track foreground window for active pin green dot
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const fg = await invoke<{
          isSelf?: boolean;
          windowId?: string | null;
        }>("get_foreground_app");
        if (cancelled) return;
        if (fg.isSelf) return;
        const id = fg.windowId ?? null;
        if (id === lastFgId.current) return;
        lastFgId.current = id;
        setActiveWindowId(id);
      } catch {
        /* noop */
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 450);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void (async () => {
      try {
        const list = await invoke<Record<string, unknown>[]>("hub_shortcuts_list_pins");
        if (!cancelled) setPins((list || []).map(normalizePin));
        unsubs.push(
          await listen<Record<string, unknown>[]>("shortcuts-pins-changed", (ev) => {
            if (!cancelled) setPins((ev.payload || []).map(normalizePin));
          }),
        );
        unsubs.push(
          await listen<{ pluginId: string; badge: string | number | null }>(
            "shortcuts-badge-changed",
            (ev) => {
              if (cancelled || !ev.payload?.pluginId) return;
              pluginRegistry.setBadge(ev.payload.pluginId, ev.payload.badge ?? null);
            },
          ),
        );
        unsubs.push(
          await listen<string>("plugin-popup-opened", (ev) => {
            if (!cancelled) {
              popupOpenRef.current = true;
              setPopupOpen(true);
              setPopupPluginId(typeof ev.payload === "string" ? ev.payload : null);
            }
          }),
        );
        unsubs.push(
          await listen("plugin-popup-closed", () => {
            if (!cancelled) {
              popupOpenRef.current = false;
              setPopupOpen(false);
              setPopupPluginId(null);
            }
          }),
        );
      } catch (err) {
        console.error("[ShortcutsHost]", err);
      }
    })();
    return () => {
      cancelled = true;
      clearHoverTimer();
      unsubs.forEach((fn) => fn());
    };
  }, []);

  const openPopupAt = useCallback(async (pluginId: string, anchorKey: string) => {
    const el = anchorRefs.current.get(anchorKey);
    if (!el || openingRef.current) return;
    openingRef.current = true;
    try {
      const { x, y } = await popupAnchor(el);
      await invoke("open_plugin_popup", { pluginId, x, y });
    } catch (err) {
      console.error("[ShortcutsHost] open popup failed", err);
    } finally {
      openingRef.current = false;
    }
  }, []);

  const scheduleOpen = (pluginId: string, anchorKey: string) => {
    clearHoverTimer();
    if (popupOpenRef.current && popupPluginId === pluginId) return;
    hoverTimerRef.current = setTimeout(() => {
      hoverTimerRef.current = null;
      void openPopupAt(pluginId, anchorKey);
    }, HOVER_OPEN_MS);
  };

  const onPinClick = async (pin: HostShortcutPin) => {
    clearHoverTimer();
    if (pin.action === "focus.window" && pin.windowId) {
      try {
        await invoke("hub_windows_focus", {
          pluginId: pin.pluginId,
          id: pin.windowId,
        });
      } catch (err) {
        console.error(err);
        void openPopupAt(pin.pluginId, `pin:${pin.pluginId}:${pin.id}`);
      }
      return;
    }
    void openPopupAt(pin.pluginId, `pin:${pin.pluginId}:${pin.id}`);
  };

  const onDividerPointerDown = (
    e: ReactPointerEvent<HTMLDivElement>,
    pin: HostShortcutPin,
  ) => {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    e.stopPropagation();
    clearHoverTimer();
    dragRef.current = {
      pluginId: pin.pluginId,
      pinId: pin.id,
      startX: e.clientX,
      originW: pin.width,
    };
    setResizing(true);
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onDividerPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const width = Math.max(
      PIN_MIN_W,
      Math.min(PIN_MAX_W, drag.originW + (e.clientX - drag.startX)),
    );
    setPins((prev) =>
      prev.map((p) =>
        p.pluginId === drag.pluginId && p.id === drag.pinId ? { ...p, width } : p,
      ),
    );
  };

  const onDividerPointerUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    setResizing(false);
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    const pin = pins.find((p) => p.pluginId === drag.pluginId && p.id === drag.pinId);
    if (!pin) return;
    void invoke("hub_shortcuts_resize_pin", {
      pluginId: drag.pluginId,
      pinId: drag.pinId,
      width: pin.width,
    }).catch(console.error);
  };

  const plugins = pluginRegistry.listShortcuts();
  const activePlugin =
    expandedPluginId != null ? plugins.find((p) => p.pluginId === expandedPluginId) : null;

  const runAction = (p: ShortcutsPluginRuntime) => {
    const action = p.manifest.slots?.shortcuts?.action ?? "popup.open";
    if (action === "popup.open") {
      clearHoverTimer();
      if (popupOpenRef.current && popupPluginId === p.pluginId) {
        void invoke("close_plugin_popup").catch(() => undefined);
        return;
      }
      void openPopupAt(p.pluginId, `chip:${p.pluginId}`);
      return;
    }
    if (action === "expand") {
      const next = expandedPluginId === p.pluginId ? null : p.pluginId;
      pluginRegistry.setExpanded(next);
      setExpandedPluginId(next);
    }
  };

  if (bounds.maxExpandWidth < 48 || (plugins.length === 0 && pins.length === 0)) {
    return (
      <div
        ref={hostRef}
        className="shortcuts-host is-empty"
        style={{ left: bounds.x, width: 0, height: SHORTCUTS_HEIGHT }}
        aria-hidden
      />
    );
  }

  return (
    <div
      ref={hostRef}
      className={`shortcuts-host${popupOpen ? " is-popup-open" : ""}${resizing ? " is-resizing" : ""}`}
      style={{
        left: bounds.x,
        width: "auto",
        maxWidth: bounds.maxExpandWidth,
        height: SHORTCUTS_HEIGHT,
      }}
      data-bounds-w={bounds.maxExpandWidth}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="shortcuts-collapsed" role="toolbar" aria-label="快捷区">
        {plugins.map((p) => {
          const config = p.manifest.slots?.shortcuts;
          const label = config?.label ?? p.manifest.name;
          const active =
            (popupOpen && popupPluginId === p.pluginId) ||
            (expandedPluginId === p.pluginId && activePlugin != null);
          return (
            <button
              key={p.pluginId}
              type="button"
              className={`shortcuts-chip${pins.length ? " is-manage" : ""}${active ? " is-active" : ""}`}
              aria-label={label}
              ref={(el) => {
                const key = `chip:${p.pluginId}`;
                if (el) anchorRefs.current.set(key, el);
                else anchorRefs.current.delete(key);
              }}
              onPointerEnter={() => {
                if ((config?.action ?? "popup.open") === "popup.open") {
                  scheduleOpen(p.pluginId, `chip:${p.pluginId}`);
                }
              }}
              onPointerLeave={clearHoverTimer}
              onClick={() => runAction(p)}
            >
              <PluginIcon icon={config?.icon} />
              {!pins.length ? (
                <span className="shortcuts-chip-label">{truncate(label, 4)}</span>
              ) : null}
              {!pins.length && p.badge != null ? (
                <span className="shortcuts-badge">{p.badge}</span>
              ) : null}
            </button>
          );
        })}

        {pins.map((pin, index) => {
          const isFg = !!(activeWindowId && pin.windowId === activeWindowId);
          const autoW = pinWidthForLabel(pin.label, {
            activeDot: isFg,
            badge: pin.badge != null,
          });
          return (
            <div key={`${pin.pluginId}:${pin.id}`} className="shortcuts-pin-wrap">
              {index === 0 && plugins.length > 0 ? (
                <div className="shortcuts-pin-divider is-static" aria-hidden />
              ) : null}
              <button
                type="button"
                className={`shortcuts-chip is-pin${popupOpen && popupPluginId === pin.pluginId ? " is-active" : ""}${isFg ? " is-fg" : ""}`}
                style={{ width: autoW, maxWidth: autoW }}
                aria-label={pin.label}
                ref={(el) => {
                  const key = `pin:${pin.pluginId}:${pin.id}`;
                  if (el) anchorRefs.current.set(key, el);
                  else anchorRefs.current.delete(key);
                }}
                onPointerEnter={() => {
                  if (pin.action === "popup.open") {
                    scheduleOpen(pin.pluginId, `pin:${pin.pluginId}:${pin.id}`);
                  }
                }}
                onPointerLeave={clearHoverTimer}
                onClick={() => void onPinClick(pin)}
              >
                {isFg ? <span className="shortcuts-active-dot" aria-hidden /> : null}
                <span className="shortcuts-chip-label">{pin.label}</span>
                {pin.badge != null ? <span className="shortcuts-badge">{pin.badge}</span> : null}
              </button>
              {index < pins.length - 1 ? (
                <div className="shortcuts-pin-divider is-static" aria-hidden />
              ) : null}
            </div>
          );
        })}
      </div>

      {activePlugin ? (
        <div className="shortcuts-expanded">
          {activePlugin.items.length ? (
            activePlugin.items.map((item) => (
              <button
                key={item.id}
                type="button"
                className="shortcuts-chip"
                onClick={() => {
                  pluginRegistry.setExpanded(null);
                  setExpandedPluginId(null);
                }}
              >
                <span className="shortcuts-chip-label">{item.title}</span>
              </button>
            ))
          ) : (
            <span className="shortcuts-empty">暂无展开项</span>
          )}
        </div>
      ) : null}
    </div>
  );
}
