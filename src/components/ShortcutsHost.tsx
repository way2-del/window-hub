import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  hideChromeHoverTip,
  hostTipPointerProps,
  showChromeHoverTip,
} from "../chromeHoverTip";
import {
  moveIdInOrder,
  pickDropTarget,
  sameOrder,
  sortByOrderKey,
} from "../chromeReorder";
import {
  SHORTCUTS_HEIGHT,
  computeShortcutsBounds,
  type ShortcutsBounds,
} from "../plugins/shortcutsGeometry";
import { pluginRegistry } from "../plugins/registry";
import type { ShortcutsPluginRuntime } from "../plugins/types";
import ShortcutsPluginStrip, {
  type ShortcutsHoverTip,
} from "./ShortcutsPluginStrip";
import { WH_SHORTCUTS_EVT } from "../plugins/shortcutsHubBridge";
import {
  parseScopes,
  shortcutsScopeVisible,
  type ShortcutsPluginScope,
} from "../shortcutsPrefs";
import "./ShortcutsHost.css";

const POPUP_GAP = 8;
/** Intentional dwell before hover-opens popup — short values flash on every pass. */
const HOVER_OPEN_MS = 450;
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

/** Host 仅在 manage=settings 时画 2×2；custom 由插件自画，none/缺省不画 */
function shouldShowHostSettingsChip(p: ShortcutsPluginRuntime): boolean {
  return (p.manifest.slots?.shortcuts?.manage ?? "none") === "settings";
}

/**
 * 岛栏 worker：有 island.bar + shortcuts 入口，且 manage 不为 custom/settings。
 * 只跑轮询/setBar，不得占用快捷区横向空间（否则左侧标题↔首个 chip 间距会跟着变）。
 */
function isIslandBarWorker(p: ShortcutsPluginRuntime): boolean {
  const manage = p.manifest.slots?.shortcuts?.manage ?? "none";
  if (manage === "custom" || manage === "settings") return false;
  return Boolean(
    p.manifest.slots?.["island.bar"] &&
      hasShortcutsEntry(p) &&
      (p.manifest.capabilities ?? []).includes("island.bar"),
  );
}

function ManageIcon() {
  return (
    <svg
      className="shortcuts-chip-icon"
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      aria-hidden
    >
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  );
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
  const [pluginOrder, setPluginOrder] = useState<string[]>([]);
  const [scopes, setScopes] = useState<Record<string, ShortcutsPluginScope>>({});
  const [fgExe, setFgExe] = useState<{
    exe?: string | null;
    exe_name?: string | null;
  } | null>(null);
  const [ctrlHeld, setCtrlHeld] = useState(false);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropHint, setDropHint] = useState<{ toId: string; place: "before" | "after" } | null>(
    null,
  );
  const [stripWidths, setStripWidths] = useState<Record<string, number>>({});
  const [hoverTip, setHoverTip] = useState<ShortcutsHoverTip | null>(null);
  const dragIdRef = useRef<string | null>(null);
  const dropHintRef = useRef<{ toId: string; place: "before" | "after" } | null>(null);
  const pluginOrderRef = useRef<string[]>([]);
  const exclusiveRef = useRef<string | null>(null);
  pluginOrderRef.current = pluginOrder;
  exclusiveRef.current = exclusivePluginId;
  dragIdRef.current = dragId;
  dropHintRef.current = dropHint;

  useEffect(() => {
    return () => {
      void hideChromeHoverTip();
    };
  }, []);

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
  }, [recomputeBounds, exclusivePluginId, popupOpen, stripWidths, scopes, fgExe]);

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
    type Prefs = {
      exclusivePluginId?: string | null;
      pluginOrder?: string[] | null;
      scopes?: Record<string, ShortcutsPluginScope> | null;
    };
    void (async () => {
      try {
        const prefs = await invoke<Prefs>("get_shortcuts_prefs");
        if (!cancelled) {
          setExclusivePluginId(prefs.exclusivePluginId ?? null);
          setPluginOrder(Array.isArray(prefs.pluginOrder) ? prefs.pluginOrder : []);
          setScopes(parseScopes(prefs.scopes));
        }
      } catch {
        /* noop */
      }
    })();
    let un: (() => void) | undefined;
    void listen<Prefs>("shortcuts-prefs", (ev) => {
      if (cancelled) return;
      setExclusivePluginId(ev.payload?.exclusivePluginId ?? null);
      if (Array.isArray(ev.payload?.pluginOrder)) {
        setPluginOrder(ev.payload.pluginOrder);
      }
      if (ev.payload?.scopes !== undefined) {
        setScopes(parseScopes(ev.payload.scopes));
      }
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
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Control") {
        setCtrlHeld(true);
        setHoverTip(null);
        void hideChromeHoverTip();
        clearHoverTimer();
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
      if (dragIdRef.current) {
        setDragId(null);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // While Ctrl-reorder / dragging: kill tips / hover-open and close popup so they cannot steal pointer.
  useEffect(() => {
    if (!(ctrlHeld || dragId)) return;
    setHoverTip(null);
    void hideChromeHoverTip();
    clearHoverTimer();
    if (popupOpenRef.current) {
      void invoke("close_plugin_popup").catch(() => undefined);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctrlHeld, dragId]);

  useEffect(() => {
    if (!hoverTip || !hoverTip.lines.length) {
      void hideChromeHoverTip();
      return;
    }
    if (ctrlHeld || dragIdRef.current) {
      void hideChromeHoverTip();
      return;
    }
    void showChromeHoverTip({
      lines: hoverTip.lines,
      x: hoverTip.x,
      y: hoverTip.y,
    });
  }, [hoverTip, ctrlHeld]);

  // 全 Host 共用一次前台轮询，广播到各插件 iframe（含非 worker）
  useEffect(() => {
    let cancelled = false;
    let lastWid: string | null | undefined = undefined;
    const broadcast = (msg: Record<string, unknown>) => {
      const root = hostRef.current;
      if (!root) return;
      root.querySelectorAll("iframe").forEach((el) => {
        (el as HTMLIFrameElement).contentWindow?.postMessage(msg, "*");
      });
    };
    const tick = async () => {
      try {
        const fg = await invoke<{
          isSelf?: boolean;
          windowId?: string | null;
          exeName?: string | null;
          exe_name?: string | null;
        }>("get_foreground_app");
        if (cancelled) return;
        if (fg.isSelf) return;
        const wid = fg.windowId ?? null;
        const exeName = fg.exeName ?? fg.exe_name ?? null;
        setFgExe((prev) => {
          const next = { exe_name: exeName };
          if (prev?.exe_name === next.exe_name) return prev;
          return next;
        });
        if (wid === lastWid) return;
        lastWid = wid;
        broadcast({
          channel: WH_SHORTCUTS_EVT,
          type: "foreground-changed",
          windowId: wid,
          exeName,
        });
      } catch {
        /* noop */
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  // island-prefs：Host 只 listen 一次，广播到各 iframe；启动时先推一版当前 prefs
  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    const broadcast = (msg: Record<string, unknown>) => {
      const root = hostRef.current;
      if (!root) return;
      root.querySelectorAll("iframe").forEach((el) => {
        (el as HTMLIFrameElement).contentWindow?.postMessage(msg, "*");
      });
    };
    const pushPrefs = (prefs: Record<string, unknown>) => {
      broadcast({
        channel: WH_SHORTCUTS_EVT,
        type: "island-prefs",
        prefs,
      });
    };
    void invoke<Record<string, unknown>>("get_island_prefs")
      .then((prefs) => {
        if (!cancelled) pushPrefs(prefs ?? {});
      })
      .catch(() => undefined);
    void (async () => {
      try {
        unsubs.push(
          await listen<Record<string, unknown>>("island-prefs", (ev) => {
            if (cancelled) return;
            pushPrefs(ev.payload ?? {});
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
              setHoverTip(null);
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

  const openSettingsForPlugin = useCallback(async (pluginId: string) => {
    try {
      await invoke("open_settings_window", { pluginId });
    } catch (err) {
      console.error("[ShortcutsHost] open settings failed", err);
    }
  }, []);

  const scheduleOpen = (pluginId: string, anchorKey: string) => {
    if (ctrlHeld || dragIdRef.current) return;
    clearHoverTimer();
    if (popupOpenRef.current && popupPluginId === pluginId) return;
    hoverTimerRef.current = setTimeout(() => {
      hoverTimerRef.current = null;
      if (ctrlHeld || dragIdRef.current) return;
      void openPopupAt(pluginId, anchorKey);
    }, HOVER_OPEN_MS);
  };

  const onRequestWidth = useCallback((pluginId: string, width: number) => {
    const raw = Math.round(width || 0);
    const rec = pluginRegistry.get(pluginId);
    // 岛栏 worker 永不占位；其它条仍受 MIN_STRIP_W 约束
    const next = rec && isIslandBarWorker(rec) ? 0 : raw <= 0 ? 0 : Math.max(MIN_STRIP_W, raw);
    setStripWidths((prev) => {
      if (prev[pluginId] === next) return prev;
      return { ...prev, [pluginId]: next };
    });
  }, []);

  const persistPluginOrder = useCallback(async (nextOrder: string[]) => {
    setPluginOrder(nextOrder);
    try {
      await invoke("set_shortcuts_prefs", {
        prefs: {
          exclusivePluginId: exclusiveRef.current,
          pluginOrder: nextOrder,
        },
      });
    } catch (err) {
      console.error("[ShortcutsHost] persist order", err);
    }
  }, []);

  const finishReorder = useCallback(
    (fromId: string, hint: { toId: string; place: "before" | "after" } | null) => {
      setDragId(null);
      setDropHint(null);
      dropHintRef.current = null;
      dragIdRef.current = null;
      if (!hint) return;
      const visible = Array.from(
        hostRef.current?.querySelectorAll<HTMLElement>("[data-plugin-id]:not([data-bar-worker])") ?? [],
      )
        .map((el) => el.dataset.pluginId || "")
        .filter(Boolean);
      const base =
        pluginOrderRef.current.length > 0
          ? [...pluginOrderRef.current]
          : [...visible];
      for (const id of visible) {
        if (!base.includes(id)) base.push(id);
      }
      const next = moveIdInOrder(base, fromId, hint.toId, hint.place);
      if (sameOrder(base, next)) return;
      void persistPluginOrder(next);
    },
    [persistPluginOrder],
  );

  const onReorderPointerDown = useCallback(
    (pluginId: string, e: ReactPointerEvent<HTMLElement>) => {
      if (!e.ctrlKey || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      setHoverTip(null);
      void hideChromeHoverTip();
      clearHoverTimer();
      setDragId(pluginId);
      setDropHint(null);
      dropHintRef.current = null;
      dragIdRef.current = pluginId;

      const onMove = (ev: PointerEvent) => {
        if (!dragIdRef.current) return;
        const root = hostRef.current?.querySelector(".shortcuts-collapsed");
        if (!root) return;
        const units = Array.from(
          root.querySelectorAll<HTMLElement>("[data-plugin-id]:not([data-bar-worker])"),
        )
          .map((el) => {
            const r = el.getBoundingClientRect();
            return { id: el.dataset.pluginId || "", left: r.left, width: r.width };
          })
          .filter((u) => u.id);
        const hint = pickDropTarget(ev.clientX, units, dragIdRef.current);
        dropHintRef.current = hint;
        setDropHint(hint);
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        const fromId = dragIdRef.current;
        const hint = dropHintRef.current;
        if (fromId) finishReorder(fromId, hint);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [finishReorder],
  );

  const onHoverTipSafe = useCallback(
    (tip: ShortcutsHoverTip | null) => {
      if (ctrlHeld || dragIdRef.current) {
        if (tip) return;
        setHoverTip(null);
        return;
      }
      setHoverTip(tip);
    },
    [ctrlHeld],
  );

  const pluginsAll = pluginRegistry.listShortcuts();
  // 独占某插件时仍挂载「岛栏 worker」：声明 island.bar + entry.shortcuts 的隐形条（如天气）
  const pluginsExclusive = exclusivePluginId
    ? pluginsAll.filter((p) => {
        if (p.pluginId === exclusivePluginId) return true;
        const m = pluginRegistry.get(p.pluginId)?.manifest;
        return Boolean(m?.slots?.["island.bar"] && m.entry?.shortcuts);
      })
    : pluginsAll;

  const plugins = pluginsExclusive.filter((p) => {
    if (isIslandBarWorker(p)) return true;
    return shortcutsScopeVisible(scopes, p.pluginId, fgExe);
  });

  const pluginsSorted = sortByOrderKey(plugins, pluginOrder, (p) => p.pluginId);

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

  const reorderMode = ctrlHeld || dragId != null;

  return (
    <div
      ref={hostRef}
      className={`shortcuts-host${popupOpen ? " is-popup-open" : ""}${
        reorderMode ? " is-reorder" : ""
      }${dragId ? " is-dragging" : ""}`}
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
        {pluginsSorted.map((p) => {
          const entry = hasShortcutsEntry(p);
          const label = p.manifest.slots?.shortcuts?.label ?? p.manifest.name;
          const showSettingsChip = shouldShowHostSettingsChip(p);
          const manageKey = `manage:${p.pluginId}`;
          const unitClass = [
            "shortcuts-strip-unit",
            dragId === p.pluginId ? "is-dragging" : "",
            dropHint?.toId === p.pluginId ? `is-drop-${dropHint.place}` : "",
          ]
            .filter(Boolean)
            .join(" ");

          if (entry) {
            const worker = isIslandBarWorker(p);
            const requested = worker ? 0 : stripWidths[p.pluginId] ?? DEFAULT_STRIP_W;
            return (
              <div
                key={p.pluginId}
                className={`${unitClass}${worker ? " is-bar-worker" : ""}`}
                data-plugin-id={p.pluginId}
                {...(worker ? { "data-bar-worker": "" } : {})}
                aria-hidden={worker || undefined}
              >
                {showSettingsChip ? (
                  <button
                    type="button"
                    className="shortcuts-chip is-manage"
                    aria-label={`${label}设置`}
                    {...(reorderMode ? {} : hostTipPointerProps(`${label}设置`))}
                    ref={(el) => {
                      if (el) anchorRefs.current.set(manageKey, el);
                      else anchorRefs.current.delete(manageKey);
                    }}
                    onClick={() => {
                      if (reorderMode) return;
                      clearHoverTimer();
                      void hideChromeHoverTip();
                      void openSettingsForPlugin(p.pluginId);
                    }}
                  >
                    <ManageIcon />
                  </button>
                ) : null}
                <ShortcutsPluginStrip
                  pluginId={p.pluginId}
                  entryPath={entry}
                  width={worker ? 1 : requested}
                  maxWidth={bounds.maxExpandWidth}
                  onRequestWidth={onRequestWidth}
                  onHoverTip={worker ? undefined : onHoverTipSafe}
                  barWorker={worker}
                />
                {reorderMode && !worker ? (
                  <div
                    className="shortcuts-reorder-hit"
                    aria-hidden
                    onPointerDown={(e) => onReorderPointerDown(p.pluginId, e)}
                  />
                ) : null}
              </div>
            );
          }

          const config = p.manifest.slots?.shortcuts;
          const active = popupOpen && popupPluginId === p.pluginId;
          const chipKey = `chip:${p.pluginId}`;
          if (showSettingsChip) {
            return (
              <div
                key={p.pluginId}
                className={unitClass}
                data-plugin-id={p.pluginId}
              >
                <button
                  type="button"
                  className="shortcuts-chip is-manage"
                  aria-label={`${label}设置`}
                  {...(reorderMode ? {} : hostTipPointerProps(`${label}设置`))}
                  ref={(el) => {
                    if (el) anchorRefs.current.set(manageKey, el);
                    else anchorRefs.current.delete(manageKey);
                  }}
                  onClick={() => {
                    if (reorderMode) return;
                    clearHoverTimer();
                    void hideChromeHoverTip();
                    void openSettingsForPlugin(p.pluginId);
                  }}
                >
                  <ManageIcon />
                </button>
                {reorderMode ? (
                  <div
                    className="shortcuts-reorder-hit"
                    aria-hidden
                    onPointerDown={(e) => onReorderPointerDown(p.pluginId, e)}
                  />
                ) : null}
              </div>
            );
          }
          return (
            <div
              key={p.pluginId}
              className={unitClass}
              data-plugin-id={p.pluginId}
            >
              <button
                type="button"
                className={`shortcuts-chip${active ? " is-active" : ""}`}
                aria-label={label}
                ref={(el) => {
                  if (el) anchorRefs.current.set(chipKey, el);
                  else anchorRefs.current.delete(chipKey);
                }}
                onPointerEnter={() => {
                  if (reorderMode) return;
                  if ((config?.action ?? "popup.open") === "popup.open") {
                    scheduleOpen(p.pluginId, chipKey);
                  }
                }}
                onPointerLeave={clearHoverTimer}
                onClick={() => {
                  if (reorderMode) return;
                  clearHoverTimer();
                  if (popupOpenRef.current && popupPluginId === p.pluginId) {
                    void invoke("close_plugin_popup").catch(() => undefined);
                    return;
                  }
                  void openPopupAt(p.pluginId, chipKey);
                }}
              >
                <PluginIcon icon={config?.icon} />
                <span className="shortcuts-chip-label">{truncate(label, 4)}</span>
                {p.badge != null ? <span className="shortcuts-badge">{p.badge}</span> : null}
              </button>
              {reorderMode ? (
                <div
                    className="shortcuts-reorder-hit"
                    aria-hidden
                    onPointerDown={(e) => onReorderPointerDown(p.pluginId, e)}
                  />
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
