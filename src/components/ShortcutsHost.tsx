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
import ShortcutsPluginStrip from "./ShortcutsPluginStrip";
import "./ShortcutsHost.css";

const POPUP_GAP = 8;
const HOVER_OPEN_MS = 140;
const DEFAULT_STRIP_W = 28;
const MIN_STRIP_W = 28;

type Props = {
  settingsRef: RefObject<HTMLElement | null>;
  islandWidth: number;
};

function truncate(s: string, n: number) {
  const t = s.trim();
  if (t.length <= n) return t;
  return `${t.slice(0, n - 1)}…`;
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
  return (
    <span className="shortcuts-chip-icon" aria-hidden>
      ◆
    </span>
  );
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

function hasShortcutsEntry(p: ShortcutsPluginRuntime): string | null {
  const entry = p.manifest.entry?.shortcuts;
  return entry && entry.trim() ? entry.trim() : null;
}

/**
 * Host 快捷区壳：并排挂插件 iframe 条（entry.shortcuts）；
 * 无网页入口时回退为入口 chip。固定项由插件网页自画，不用 setPins。
 */
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
  const [exclusivePluginId, setExclusivePluginId] = useState<string | null>(null);
  const [stripWidths, setStripWidths] = useState<Record<string, number>>({});

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
  }, [recomputeBounds, exclusivePluginId, popupOpen, stripWidths]);

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

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const prefs = await invoke<{ exclusivePluginId?: string | null }>("get_shortcuts_prefs");
        if (!cancelled) setExclusivePluginId(prefs.exclusivePluginId ?? null);
      } catch {
        /* noop */
      }
    })();
    let un: (() => void) | undefined;
    void listen<{ exclusivePluginId?: string | null }>("shortcuts-prefs", (ev) => {
      if (!cancelled) setExclusivePluginId(ev.payload?.exclusivePluginId ?? null);
    }).then((fn) => {
      if (cancelled) fn();
      else un = fn;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void (async () => {
      try {
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

  const onRequestWidth = useCallback((pluginId: string, width: number) => {
    const next = Math.max(MIN_STRIP_W, Math.round(width || DEFAULT_STRIP_W));
    setStripWidths((prev) => {
      if (prev[pluginId] === next) return prev;
      return { ...prev, [pluginId]: next };
    });
  }, []);

  const pluginsAll = pluginRegistry.listShortcuts();
  const plugins = exclusivePluginId
    ? pluginsAll.filter((p) => p.pluginId === exclusivePluginId)
    : pluginsAll;

  const webPlugins = plugins.filter((p) => hasShortcutsEntry(p));
  const chipPlugins = plugins.filter((p) => !hasShortcutsEntry(p));

  if (bounds.maxExpandWidth < 48 || plugins.length === 0) {
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
      className={`shortcuts-host${popupOpen ? " is-popup-open" : ""}`}
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
        {webPlugins.map((p) => {
          const entry = hasShortcutsEntry(p)!;
          const requested = stripWidths[p.pluginId] ?? DEFAULT_STRIP_W;
          return (
            <ShortcutsPluginStrip
              key={p.pluginId}
              pluginId={p.pluginId}
              entryPath={entry}
              width={requested}
              maxWidth={bounds.maxExpandWidth}
              onRequestWidth={onRequestWidth}
            />
          );
        })}

        {chipPlugins.map((p) => {
          const config = p.manifest.slots?.shortcuts;
          const label = config?.label ?? p.manifest.name;
          const active = popupOpen && popupPluginId === p.pluginId;
          return (
            <button
              key={p.pluginId}
              type="button"
              className={`shortcuts-chip${active ? " is-active" : ""}`}
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
              onClick={() => {
                clearHoverTimer();
                if (popupOpenRef.current && popupPluginId === p.pluginId) {
                  void invoke("close_plugin_popup").catch(() => undefined);
                  return;
                }
                void openPopupAt(p.pluginId, `chip:${p.pluginId}`);
              }}
            >
              <PluginIcon icon={config?.icon} />
              <span className="shortcuts-chip-label">{truncate(label, 4)}</span>
              {p.badge != null ? <span className="shortcuts-badge">{p.badge}</span> : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
