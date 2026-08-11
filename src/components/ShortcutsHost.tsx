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
  shortcutsAllowsDragOpen,
  type ShortcutsBounds,
} from "../plugins/shortcutsGeometry";
import { pluginRegistry } from "../plugins/registry";
import type { ShortcutsPluginRuntime } from "../plugins/types";
import ShortcutsPluginStrip from "./ShortcutsPluginStrip";
import "./ShortcutsHost.css";

const POPUP_GAP = 8;
/** 隐形 worker（天气/歌词）可报到 1px；可见图标条约 22 */
const MIN_STRIP_W = 1;
const DEFAULT_STRIP_W = 22;
const ICON_STRIP_W = 22;

/** Explorer → 顶栏：HTML5 dragenter 常不触发，靠 Tauri position 命中芯片 */
function hitDragOpenStrip(
  host: HTMLElement,
  logicalX: number,
  logicalY: number,
): { pluginId: string; el: HTMLElement } | null {
  const strips = host.querySelectorAll<HTMLElement>("[data-plugin]");
  for (const el of strips) {
    const pluginId = el.dataset.plugin;
    if (!pluginId || !shortcutsAllowsDragOpen(pluginId)) continue;
    const r = el.getBoundingClientRect();
    const pad = 4;
    if (
      logicalX >= r.left - pad &&
      logicalX <= r.right + pad &&
      logicalY >= r.top - pad &&
      logicalY <= r.bottom + pad
    ) {
      return { pluginId, el };
    }
  }
  return null;
}

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

  const [bounds, setBounds] = useState<ShortcutsBounds>({
    x: 0,
    width: 0,
    height: SHORTCUTS_HEIGHT,
    maxExpandWidth: 0,
  });
  const [popupOpen, setPopupOpen] = useState(false);
  const [popupPluginId, setPopupPluginId] = useState<string | null>(null);
  const popupPluginIdRef = useRef<string | null>(null);
  popupPluginIdRef.current = popupPluginId;
  const [, setRegistryVersion] = useState(0);
  const [visiblePluginIds, setVisiblePluginIds] = useState<string[] | null>(null);
  const [stripWidths, setStripWidths] = useState<Record<string, number>>({});

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
  }, [recomputeBounds, visiblePluginIds, popupOpen, stripWidths]);

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
    const applyPrefs = (prefs: {
      visiblePluginIds?: string[] | null;
      exclusivePluginId?: string | null;
    }) => {
      const ids = prefs.visiblePluginIds;
      if (Array.isArray(ids)) {
        setVisiblePluginIds(ids.length ? ids : null);
        return;
      }
      const exclusive = prefs.exclusivePluginId?.trim();
      setVisiblePluginIds(exclusive ? [exclusive] : null);
    };
    void (async () => {
      try {
        const prefs = await invoke<{
          visiblePluginIds?: string[] | null;
          exclusivePluginId?: string | null;
        }>("get_shortcuts_prefs");
        if (!cancelled) applyPrefs(prefs);
      } catch {
        /* noop */
      }
    })();
    let un: (() => void) | undefined;
    void listen<{
      visiblePluginIds?: string[] | null;
      exclusivePluginId?: string | null;
    }>("shortcuts-prefs", (ev) => {
      if (!cancelled) applyPrefs(ev.payload ?? {});
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
      unsubs.forEach((fn) => fn());
    };
  }, []);

  /** 点击：同插件已开则关闭；拖入：只保证打开，禁止 toggle 关掉 */
  const openPopupFromEl = useCallback(
    async (pluginId: string, el: HTMLElement, opts?: { forceOpen?: boolean }) => {
      if (openingRef.current) return;
      const sameOpen =
        popupOpenRef.current && popupPluginIdRef.current === pluginId;
      if (sameOpen) {
        if (opts?.forceOpen) return;
        void invoke("close_plugin_popup").catch(() => undefined);
        return;
      }
      if (opts?.forceOpen) {
        const openId = await invoke<string | null>("get_plugin_popup_id").catch(
          () => null,
        );
        if (openId === pluginId) return;
      }
      openingRef.current = true;
      try {
        // 拖放中开窗：多压制一会儿 blur，避免弹窗立刻被关掉
        await invoke("suppress_plugin_popup_blur", {
          ms: opts?.forceOpen ? 1200 : 500,
        }).catch(() => undefined);
        const { x, y } = await popupAnchor(el);
        await invoke("open_plugin_popup", {
          pluginId,
          x,
          y,
          forceOpen: opts?.forceOpen === true,
        });
      } catch (err) {
        console.error("[ShortcutsHost] open popup failed", err);
      } finally {
        openingRef.current = false;
      }
    },
    [],
  );

  const openPopupAt = useCallback(
    async (pluginId: string, anchorKey: string) => {
      const el = anchorRefs.current.get(anchorKey);
      if (!el) return;
      await openPopupFromEl(pluginId, el);
    },
    [openPopupFromEl],
  );

  /** 从资源管理器拖到「中转站」芯片：开弹窗；在芯片上松开则直接入库 */
  useEffect(() => {
    let un: (() => void) | undefined;
    let lastOpenAt = 0;
    void getCurrentWindow()
      .onDragDropEvent((ev) => {
        const host = hostRef.current;
        if (!host) return;
        const p = ev.payload;
        if (p.type === "leave") return;

        void (async () => {
          const win = getCurrentWindow();
          const factor = await win.scaleFactor();
          const pos = "position" in p ? p.position : null;
          if (!pos) return;
          const lx = pos.x / factor;
          const ly = pos.y / factor;
          const hit = hitDragOpenStrip(host, lx, ly);
          if (!hit) return;

          if (p.type === "enter" || p.type === "over") {
            const now = Date.now();
            if (now - lastOpenAt < 200) return;
            lastOpenAt = now;
            void openPopupFromEl(hit.pluginId, hit.el, { forceOpen: true });
            return;
          }

          if (p.type === "drop") {
            const paths = p.paths ?? [];
            if (paths.length) {
              void invoke("hub_staging_add_paths", {
                pluginId: hit.pluginId,
                paths,
              }).catch(console.error);
            }
            void openPopupFromEl(hit.pluginId, hit.el, { forceOpen: true });
          }
        })();
      })
      .then((fn) => {
        un = fn;
      })
      .catch(() => undefined);
    return () => un?.();
  }, [openPopupFromEl]);

  const onRequestWidth = useCallback((pluginId: string, width: number) => {
    const next = Math.max(MIN_STRIP_W, Math.round(width || DEFAULT_STRIP_W));
    setStripWidths((prev) => {
      if (prev[pluginId] === next) return prev;
      return { ...prev, [pluginId]: next };
    });
  }, []);

  const pluginsAll = pluginRegistry.listShortcuts();
  // 筛选时仍挂载「岛栏 worker」：声明 island.bar + entry.shortcuts 的隐形条（如天气）
  const plugins = visiblePluginIds?.length
    ? pluginsAll.filter((p) => {
        if (visiblePluginIds.includes(p.pluginId)) return true;
        const m = pluginRegistry.get(p.pluginId)?.manifest;
        return Boolean(m?.slots?.["island.bar"] && m.entry?.shortcuts);
      })
    : pluginsAll;

  const webPlugins = plugins.filter((p) => hasShortcutsEntry(p));
  const chipPlugins = plugins.filter((p) => !hasShortcutsEntry(p));

  const initialStripWidth = (p: ShortcutsPluginRuntime) => {
    const action = p.manifest.slots?.shortcuts?.action ?? "popup.open";
    // command = 隐形 worker，勿占 22/28 把标题与可见插件撑开
    if (action === "command") return 1;
    return ICON_STRIP_W;
  };

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
          const requested = stripWidths[p.pluginId] ?? initialStripWidth(p);
          return (
            <ShortcutsPluginStrip
              key={p.pluginId}
              pluginId={p.pluginId}
              entryPath={entry}
              width={requested}
              maxWidth={bounds.maxExpandWidth}
              action={p.manifest.slots?.shortcuts?.action ?? "popup.open"}
              onRequestWidth={onRequestWidth}
            />
          );
        })}

        {chipPlugins.map((p) => {
          const config = p.manifest.slots?.shortcuts;
          const label = config?.label ?? p.manifest.name;
          const iconOnly = Boolean(config?.iconOnly);
          const active = popupOpen && popupPluginId === p.pluginId;
          return (
            <button
              key={p.pluginId}
              type="button"
              className={`shortcuts-chip${active ? " is-active" : ""}${iconOnly ? " is-icon-only" : ""}`}
              aria-label={label}
              title={label}
              ref={(el) => {
                const key = `chip:${p.pluginId}`;
                if (el) anchorRefs.current.set(key, el);
                else anchorRefs.current.delete(key);
              }}
              onClick={() => {
                if (popupOpenRef.current && popupPluginId === p.pluginId) {
                  void invoke("close_plugin_popup").catch(() => undefined);
                  return;
                }
                void openPopupAt(p.pluginId, `chip:${p.pluginId}`);
              }}
            >
              <PluginIcon icon={config?.icon} />
              {iconOnly ? null : (
                <span className="shortcuts-chip-label">{truncate(label, 4)}</span>
              )}
              {p.badge != null ? <span className="shortcuts-badge">{p.badge}</span> : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
