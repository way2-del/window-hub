import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, currentMonitor } from "@tauri-apps/api/window";
import { applyGlassCss, type GlassPrefs } from "./glassPrefs";
import {
  hideChromeHoverTip,
  dockIconTipPointerProps,
  installChromeHoverTipGlobalDismiss,
} from "./chromeHoverTip";
import { DockStartIcon, DockTrashIcon, DOCK_START_BG, DOCK_TRASH_BG } from "./dockIcons";
import { useDockIconPlate } from "./dockIconPlate";
import "./DockApp.css";

type DockItem = {
  id: string;
  kind: string;
  label: string;
  matchExe: string;
  launchPath: string;
  realPath: string;
  virtualPath: string;
  iconPath: string;
  uwp: boolean;
  iconPng?: string | null;
  iconScale?: number;
  iconOffsetX?: number;
  iconOffsetY?: number;
  /** Solid plate behind glyph (`#RRGGBB` / `transparent`). Empty = auto. */
  iconBg?: string;
};

type DockPrefs = {
  enabled: boolean;
  displayMode: string;
  hideSystemTaskbar: boolean;
  items: DockItem[];
  hotkey: string;
  magnification?: number;
  cornerRadiusPx?: number;
  hiddenItemIds?: string[];
};

type HubWindow = {
  id: string;
  hwnd: number;
  title: string;
  exe?: string | null;
  exeName?: string | null;
};

const STATUS_MENU_W = 200;
const STATUS_MENU_H = 340;
const STATUS_MENU_GAP = 8;
const STATUS_MENU_MARGIN = 8;

/** Icon slot width (matches CSS / Rust DOCK_ICON). */
const ICON_SLOT = 40;
const ICON_GAP = 6;
const BAR_PAD_X_MIN = 2;
const SEP_W = 10;
/** How many icon-widths the fan reaches on each side. */
const MAG_RANGE = 2.25;
/** Fixed magnification — not user-configurable (matches Rust DOCK_MAG_SCALE). */
const DOCK_MAG = 1.6;

/** Match Rust `dock_pad_x` — keep glyphs inside large capsule corners. */
function dockPadX(cornerRadiusPx: number): number {
  return Math.min(16, Math.max(BAR_PAD_X_MIN, cornerRadiusPx * 0.5));
}

function fanScale(distancePx: number, maxScale: number): number {
  if (maxScale <= 1.001) return 1;
  const reach = ICON_SLOT * MAG_RANGE;
  if (reach <= 0 || distancePx >= reach) return 1;
  const t = distancePx / reach;
  const w = Math.cos((t * Math.PI) / 2);
  return 1 + (maxScale - 1) * w * w;
}

/** Resting (unscaled) centers relative to bar content left — avoids layout feedback. */
function restingCenters(items: DockItem[], padX: number): Map<string, number> {
  const map = new Map<string, number>();
  let x = padX;
  items.forEach((item, i) => {
    if (i > 0) x += ICON_GAP;
    if (item.kind === "separator") {
      x += SEP_W;
      return;
    }
    map.set(item.id, x + ICON_SLOT / 2);
    x += ICON_SLOT;
  });
  return map;
}

/** Resting bar width — used to map pointer X into unscaled layout space. */
function restingBarWidth(items: DockItem[], padX: number): number {
  let w = padX * 2;
  items.forEach((item, i) => {
    if (i > 0) w += ICON_GAP;
    w += item.kind === "separator" ? SEP_W : ICON_SLOT;
  });
  return Math.max(120, w);
}

function exeMatches(item: DockItem, w: HubWindow): boolean {
  const want = (item.matchExe || "").toLowerCase();
  if (!want) return false;
  const name = (w.exeName || "").toLowerCase();
  const path = (w.exe || "").toLowerCase();
  const real = (item.realPath || "").toLowerCase();
  if (name && (name === want || `${name}.exe` === want || name === want.replace(/\.exe$/, ""))) {
    return true;
  }
  if (real && path && path === real) return true;
  return false;
}

/** Prefer above the click (dock is bottom); clamp into the current monitor. */
async function openStatusMenuAtClientPoint(
  clientX: number,
  clientY: number,
  itemId?: string | null,
) {
  const win = getCurrentWindow();
  const [factor, outer, monitor] = await Promise.all([
    win.scaleFactor(),
    win.outerPosition(),
    currentMonitor(),
  ]);
  const originX = outer.x / factor;
  const originY = outer.y / factor;
  let x = originX + clientX;
  // Dock sits on the bottom edge — open upward first.
  let y = originY + clientY - STATUS_MENU_H - STATUS_MENU_GAP;
  const pinBottom = originY + clientY - STATUS_MENU_GAP;

  if (monitor) {
    const mx = monitor.position.x / factor;
    const my = monitor.position.y / factor;
    const mw = monitor.size.width / factor;
    const mh = monitor.size.height / factor;
    const minX = mx + STATUS_MENU_MARGIN;
    const maxX = mx + mw - STATUS_MENU_W - STATUS_MENU_MARGIN;
    const minY = my + STATUS_MENU_MARGIN;
    const maxY = my + mh - STATUS_MENU_H - STATUS_MENU_MARGIN;

    if (y < minY) {
      y = originY + clientY + STATUS_MENU_GAP;
    }
    x = Math.min(Math.max(minX, x), Math.max(minX, maxX));
    y = Math.min(Math.max(minY, y), Math.max(minY, maxY));
  } else {
    x = Math.max(STATUS_MENU_MARGIN, x);
    y = Math.max(STATUS_MENU_MARGIN, y);
  }

  const visible = await invoke<boolean>("is_status_menu_popup_open");
  if (visible) {
    await invoke("close_status_menu_popup");
  }
  await invoke("open_status_menu_popup", {
    x,
    y,
    fromDock: true,
    itemId: itemId?.trim() || null,
    pinBottom,
  });
}

function DockRasterGlyph({ src }: { src: string }) {
  return <img className="dock-icon" src={src} alt="" draggable={false} />;
}

function DockItemGlyph({
  item,
  label,
  scale,
}: {
  item: DockItem;
  label: string;
  scale: number;
}) {
  const iconScale = typeof item.iconScale === "number" && item.iconScale > 0 ? item.iconScale : 1;
  const ox = item.iconOffsetX ?? 0;
  const oy = item.iconOffsetY ?? 0;
  const bgRaw = (item.iconBg ?? "").trim();
  const src = item.iconPng ? `data:image/png;base64,${item.iconPng}` : null;
  const autoPlate = useDockIconPlate(src);

  let plateClass = "dock-icon-tile";
  let plateBg: string | undefined;
  if (bgRaw === "transparent" || bgRaw === "none") {
    plateClass += " has-bg";
    plateBg = "transparent";
  } else if (bgRaw) {
    plateClass += " has-custom-bg";
    plateBg = bgRaw;
  } else if (item.kind === "startmenu" && !item.iconPng) {
    plateClass += " has-custom-bg";
    plateBg = DOCK_START_BG;
  } else if (item.kind === "trash" && !item.iconPng) {
    plateClass += " has-custom-bg";
    plateBg = DOCK_TRASH_BG;
  } else if (autoPlate || !src) {
    plateClass += " needs-plate";
  } else {
    plateClass += " has-bg";
  }

  const glyph =
    item.kind === "startmenu" && !item.iconPng ? (
      <DockStartIcon />
    ) : item.kind === "trash" && !item.iconPng ? (
      <DockTrashIcon />
    ) : src ? (
      <DockRasterGlyph src={src} />
    ) : (
      <span className="dock-icon-fallback" aria-hidden>
        {(label || "?").charAt(0)}
      </span>
    );

  return (
    <span
      className={plateClass}
      style={
        {
          ["--dock-scale" as string]: String(scale),
          ...(plateBg != null ? { background: plateBg } : null),
        } as CSSProperties
      }
    >
      <span
        className="dock-icon-glyph"
        style={{
          transform: `translate(${ox * scale}px, ${oy * scale}px) scale(${iconScale})`,
        }}
      >
        {glyph}
      </span>
    </span>
  );
}

export default function DockApp() {
  const [prefs, setPrefs] = useState<DockPrefs | null>(null);
  /** Pinned + running-not-pinned (before trash); may differ from prefs.items. */
  const [displayItems, setDisplayItems] = useState<DockItem[]>([]);
  const [windows, setWindows] = useState<HubWindow[]>([]);
  /** Overflow compact toast. */
  const [compactTip, setCompactTip] = useState<string | null>(null);
  /** Pointer X relative to `.dock-bar` content box. */
  const [localX, setLocalX] = useState<number | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef(0);
  const expandedRef = useRef(false);
  const expandInflightRef = useRef<boolean | null>(null);
  /** Fan only after HWND widen succeeds — never grow icons on resting width. */
  const fanArmedRef = useRef(false);
  const [fanArmed, setFanArmed] = useState(false);
  /** Extend stack hit height through headroom while hovering. */
  const [fanLive, setFanLive] = useState(false);
  /** Bumped to cancel a pending armFanAfterWidenPaint. */
  const fanArmGenRef = useRef(0);
  const pendingFanXRef = useRef<number | null>(null);
  const collapseTimerRef = useRef<number | null>(null);
  const layoutSigRef = useRef("");
  const winExeSigRef = useRef("");
  const winRunSigRef = useRef("");

  const cancelCollapseTimer = () => {
    if (collapseTimerRef.current != null) {
      window.clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
  };

  const disarmFan = () => {
    fanArmGenRef.current += 1;
    fanArmedRef.current = false;
    setFanArmed(false);
    pendingFanXRef.current = null;
    setLocalX(null);
  };

  /** Only after HWND widen is committed + painted — never grow icons on resting width. */
  const armFanAfterWidenPaint = () => {
    const gen = fanArmGenRef.current;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (gen !== fanArmGenRef.current) return;
        if (!expandedRef.current) return;
        if (expandInflightRef.current === false) return;
        fanArmedRef.current = true;
        setFanArmed(true);
        const pending = pendingFanXRef.current;
        if (pending != null) setLocalX(pending);
      });
    });
  };

  const armFan = () => {
    fanArmedRef.current = true;
    setFanArmed(true);
    const pending = pendingFanXRef.current;
    if (pending != null) {
      setLocalX(pending);
    }
  };

  const setExpanded = (next: boolean) => {
    // Already widened this hover session — do not re-invoke snap / restore.
    if (expandedRef.current === next) {
      if (next && !fanArmedRef.current) armFanAfterWidenPaint();
      return;
    }
    if (expandInflightRef.current === next) return;
    expandInflightRef.current = next;
    if (next) {
      // Phase 1: widen only — fan stays off until invoke + paint.
      disarmFan();
    }
    void invoke<boolean>("dock_set_hover_expand", { expanded: next })
      .then((ok) => {
        if (ok) {
          expandedRef.current = next;
          if (next) {
            // Phase 2: HWND is wide — paint, then allow magnification.
            armFanAfterWidenPaint();
          } else {
            disarmFan();
          }
        } else if (next) {
          expandInflightRef.current = null;
          window.setTimeout(() => {
            if (!expandedRef.current) setExpanded(true);
          }, 32);
          return;
        }
        if (expandInflightRef.current === next) {
          expandInflightRef.current = null;
        }
      })
      .catch(() => {
        if (expandInflightRef.current === next) {
          expandInflightRef.current = null;
        }
      });
  };

  const queueFanFromClientX = (clientX: number) => {
    const bar = barRef.current;
    if (!bar) return;
    const rect = bar.getBoundingClientRect();
    const padX = dockPadX(prefs?.cornerRadiusPx ?? 20);
    const restW = restingBarWidth(displayItems, padX);
    // Bar is full HWND width; icons are centered — map into resting content space.
    const offset = (rect.width - restW) / 2;
    const x = clientX - rect.left - offset;
    // Buffer pointer until widen completes — never drive scales early.
    if (!fanArmedRef.current || !expandedRef.current) {
      pendingFanXRef.current = x;
      return;
    }
    setLocalX(x);
  };

  const beginCollapseAfterFanRest = () => {
    // Un-magnify first (~160ms CSS), then animate HWND width down (same ~180ms
    // ease as widen) — never snap-shrink the glass.
    setFanLive(false);
    disarmFan();
    cancelCollapseTimer();
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = null;
      if (expandedRef.current) setExpanded(false);
    }, 160);
  };

  useEffect(() => installChromeHoverTipGlobalDismiss(), []);

  useEffect(() => {
    let cancelled = false;
    const applyMaterial = async () => {
      try {
        const material = await invoke<GlassPrefs>("get_material_prefs");
        const sysDark = await invoke<boolean>("system_apps_dark").catch(() => undefined);
        applyGlassCss(
          {
            kind: "mica-alt",
            dark: material.dark,
            acrylicAlpha: material.acrylicAlpha,
          },
          sysDark,
        );
        await invoke("apply_window_effect", {}).catch(() => undefined);
      } catch {
        /* noop */
      }
    };
    void applyMaterial();
    const retryA = window.setTimeout(() => {
      void applyMaterial();
    }, 150);
    const retryB = window.setTimeout(() => {
      void applyMaterial();
    }, 400);

    const refreshDisplay = async () => {
      try {
        const items = await invoke<DockItem[]>("get_dock_display_items");
        if (cancelled) return;
        const sig = items.map((i) => `${i.id}:${i.kind}`).join("|");
        const changed = sig !== layoutSigRef.current;
        layoutSigRef.current = sig;
        setDisplayItems(items);
        if (changed) {
          await invoke("dock_relayout").catch(() => undefined);
        }
      } catch (e) {
        console.error(e);
      }
    };

    const applyWindowList = (list: HubWindow[], allowDisplayRefresh: boolean) => {
      if (cancelled) return;
      // Ignore title-only flaps (Clash / browsers) — they fire every 250ms.
      const runSig = list
        .map((w) => `${w.hwnd}:${(w.exe || "").toLowerCase()}`)
        .sort()
        .join("|");
      if (runSig !== winRunSigRef.current) {
        winRunSigRef.current = runSig;
        setWindows(list);
      }
      const exeSig = list
        .map((w) => (w.exe || "").toLowerCase())
        .filter(Boolean)
        .sort()
        .join("|");
      if (exeSig !== winExeSigRef.current) {
        winExeSigRef.current = exeSig;
        if (allowDisplayRefresh) void refreshDisplay();
      }
    };

    void (async () => {
      try {
        const p = await invoke<DockPrefs>("get_dock_prefs");
        if (!cancelled) {
          setPrefs(p);
          setDisplayItems(p.items);
        }
      } catch (e) {
        console.error(e);
      }
      try {
        const list = await invoke<HubWindow[]>("list_open_windows");
        applyWindowList(list, false);
      } catch {
        /* noop */
      }
      await refreshDisplay();
    })();

    const unsubs: Array<() => void> = [];
    void listen<DockPrefs>("dock-prefs", (e) => {
      if (!cancelled) {
        setPrefs(e.payload);
        // Pins already carry icons from the emit — merge running on top.
        setDisplayItems(e.payload.items);
      }
      void refreshDisplay();
    }).then((u) => unsubs.push(u));
    void listen<{ windows: HubWindow[] }>("hub-windows-changed", (e) => {
      // Titles flap every poll; only rebuild dock tiles when exe set changes.
      applyWindowList(e.payload?.windows ?? [], true);
    }).then((u) => unsubs.push(u));
    void listen("material-prefs", () => {
      void applyMaterial();
    }).then((u) => unsubs.push(u));
    void listen<{ newlyHidden?: number; totalHidden?: number }>("dock-compacted", (e) => {
      if (cancelled) return;
      const n = e.payload?.newlyHidden ?? 0;
      const total = e.payload?.totalHidden ?? n;
      if (n <= 0) return;
      setCompactTip(
        `空间不足，已隐藏 ${n} 个未打开图标（共 ${total}）。右键菜单可恢复。`,
      );
      void refreshDisplay();
    }).then((u) => unsubs.push(u));

    // Backup poll — dots only; display refresh gated by exe signature.
    const winTimer = window.setInterval(() => {
      void invoke<HubWindow[]>("list_open_windows")
        .then((list) => applyWindowList(list, true))
        .catch(() => undefined);
    }, 2500);

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      window.clearInterval(winTimer);
      for (const u of unsubs) u();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  useEffect(() => {
    if (!compactTip) return;
    const t = window.setTimeout(() => setCompactTip(null), 6000);
    return () => window.clearTimeout(t);
  }, [compactTip]);

  const maxScale = DOCK_MAG;
  const magOn = true;

  const activeIds = useMemo(() => {
    const set = new Set<string>();
    for (const item of displayItems) {
      if (item.kind !== "app") continue;
      // Ephemeral running:* tiles are always "on".
      if (item.id.startsWith("running:")) {
        set.add(item.id);
        continue;
      }
      if (windows.some((w) => exeMatches(item, w))) set.add(item.id);
    }
    return set;
  }, [displayItems, windows]);

  const padX = dockPadX(prefs?.cornerRadiusPx ?? 20);
  const centers = useMemo(
    () => restingCenters(displayItems, padX),
    [displayItems, padX],
  );

  const scales = useMemo(() => {
    const map = new Map<string, number>();
    // Strict order: no icon growth until dock has widened.
    if (!fanArmed || localX == null) return map;
    for (const item of displayItems) {
      if (item.kind === "separator") continue;
      const c = centers.get(item.id);
      if (c == null) {
        map.set(item.id, 1);
        continue;
      }
      map.set(item.id, fanScale(Math.abs(localX - c), maxScale));
    }
    return map;
  }, [displayItems, localX, maxScale, centers, fanArmed]);

  // AutoHide / hide snap must clear fan + collapse HWND pad.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    void listen<{ visible?: boolean }>("dock-visibility", (ev) => {
      if (ev.payload?.visible === false) {
        if (rafRef.current) cancelAnimationFrame(rafRef.current);
        cancelCollapseTimer();
        setFanLive(false);
        disarmFan();
        setExpanded(false);
      }
    }).then((u) => {
      unsub = u;
    });
    return () => unsub?.();
  }, []);

  const onBarPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!prefs) return;
    cancelCollapseTimer();
    setFanLive(true);
    // Widen at most once per hover session; moves only update fan X.
    if (!expandedRef.current && expandInflightRef.current !== true) {
      setExpanded(true);
    }
    const clientX = e.clientX;
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      queueFanFromClientX(clientX);
    });
  };

  const onBarPointerEnter = (e: ReactPointerEvent<HTMLDivElement>) => {
    cancelCollapseTimer();
    setFanLive(true);
    queueFanFromClientX(e.clientX);
    // From default width only when this session is not already wide.
    if (!expandedRef.current && expandInflightRef.current !== true) {
      setExpanded(true);
    } else if (expandedRef.current && !fanArmedRef.current) {
      armFanAfterWidenPaint();
    }
  };

  const onBarPointerLeave = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    beginCollapseAfterFanRest();
  };

  const onBarPointerCancel = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    beginCollapseAfterFanRest();
  };

  async function onItemClick(item: DockItem) {
    if (item.kind === "separator") return;
    cancelCollapseTimer();
    setFanLive(false);
    disarmFan();
    setExpanded(false);
    try {
      // Do not focus the dock HWND — Win11 may paint a native "Dock" title bar
      // into the transparent headroom on activation.
      await invoke("dock_launch_item", { itemId: item.id });
    } catch (e) {
      console.error(e);
    }
  }

  function onBackgroundContextMenu(e: MouseEvent) {
    e.preventDefault();
    const target = e.target as HTMLElement | null;
    if (target?.closest(".dock-item") || target?.closest(".dock-sep")) {
      return;
    }
    void openStatusMenuAtClientPoint(e.clientX, e.clientY).catch((err) => {
      console.error(err);
    });
  }

  if (!prefs) {
    return <div className="dock-shell dock-loading" />;
  }

  return (
    <div
      className={`dock-shell${fanLive ? " is-fan-live" : ""}`}
      data-mode={prefs.displayMode}
      data-mag={magOn ? "on" : "off"}
      onContextMenu={onBackgroundContextMenu}
    >
      {compactTip ? (
        <div className="dock-compact-tip" role="status">
          {compactTip}
        </div>
      ) : null}
      <div className="dock-stack">
        <div className="dock-chrome" aria-hidden />
        <div
          ref={barRef}
          className="dock-bar"
          style={{ paddingLeft: padX, paddingRight: padX }}
          onPointerEnter={onBarPointerEnter}
          onPointerMove={onBarPointerMove}
          onPointerLeave={onBarPointerLeave}
          onPointerCancel={onBarPointerCancel}
        >
          {displayItems.map((item) => {
            if (item.kind === "separator") {
              return <span key={item.id} className="dock-sep" aria-hidden />;
            }
            const running = activeIds.has(item.id);
            const scale = scales.get(item.id) ?? 1;
            const label =
              item.kind === "startmenu"
                ? "开始"
                : item.kind === "trash"
                  ? "回收站"
                  : item.label || item.matchExe || item.id;
            // Layout-based fan: slot grows in flex flow; sep stays unscaled.
            const slot = ICON_SLOT * scale;
            const style = {
              ["--dock-scale" as string]: String(scale),
              ["--dock-slot" as string]: `${slot}px`,
              ["--dock-hit" as string]: `${slot}px`,
            } as CSSProperties;
            return (
              <button
                key={item.id}
                type="button"
                className={`dock-item${running ? " is-running" : ""}${scale > 1.02 ? " is-magnified" : ""}`}
                {...dockIconTipPointerProps(label, { gap: 8 })}
                style={style}
                onClick={() => {
                  void hideChromeHoverTip();
                  void onItemClick(item);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  if (item.id.startsWith("running:")) return;
                  void hideChromeHoverTip();
                  void openStatusMenuAtClientPoint(e.clientX, e.clientY, item.id).catch(
                    console.error,
                  );
                }}
              >
                <span className="dock-hit">
                  <DockItemGlyph item={item} label={label} scale={scale} />
                </span>
                <span className={`dock-dot${running ? " is-on" : ""}`} aria-hidden />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
