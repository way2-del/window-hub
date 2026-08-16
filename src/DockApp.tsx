import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { flushSync } from "react-dom";
import {
  DragDropContext,
  Draggable,
  Droppable,
  type BeforeCapture,
  type DragStart,
  type DragUpdate,
  type DropResult,
} from "@hello-pangea/dnd";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, currentMonitor } from "@tauri-apps/api/window";
import { applyGlassCss, type GlassPrefs } from "./glassPrefs";
import {
  hideChromeHoverTip,
  showChromeHoverTip,
  dockIconTipPointerProps,
  installChromeHoverTipGlobalDismiss,
} from "./chromeHoverTip";
import { DockStartIcon, DockTrashIcon, DOCK_START_BG, DOCK_TRASH_BG, DOCK_AUTO_PLATE_BG } from "./dockIcons";
import { useDockIconPlate } from "./dockIconPlate";
import { plateColorFromPngBase64, peekCachedPlateColor } from "./dockIconBg";
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

/** Shared with openStatusMenuAtClientPoint (module scope) + DockApp leave handlers. */
let dockStatusMenuOpen = false;

/** Icon slot width (matches CSS / Rust DOCK_ICON). */
const ICON_SLOT = 40;
const ICON_GAP = 6;
const BAR_PAD_X_MIN = 2;
/** Visual rule is 1px; hit/layout slot is wider for drag. Keep in sync with CSS. */
const SEP_LAYOUT_W = 16;
/** Match Rust `DOCK_FAN_EXTRA` — host HWND / expanded chrome width. */
const DOCK_FAN_EXTRA = 48;
/** Chrome / Composition width tween is 240ms (DockApp.css + Rust DOCK_WIDTH_TWEEN_MS). */
/**
 * Arm fan before the glass tween fully finishes — pad already exists mid-widen,
 * so waiting the full 240ms feels laggy after AutoHide reveal.
 */
const FAN_ARM_DELAY_MS = 90;
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

/**
 * Fan keep zone = union of icon/sep hit boxes (not the full HWND-wide bar).
 * Side fan-pad is still part of `.dock-bar`, so bar left/right would keep mag
 * stuck when sliding off the leftmost/rightmost icon.
 */
function pointerInFanIconZone(clientX: number, clientY: number, bar: HTMLElement): boolean {
  const nodes = bar.querySelectorAll<HTMLElement>(".dock-hit, .dock-sep");
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  let any = false;
  for (let i = 0; i < nodes.length; i++) {
    const r = nodes[i].getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    any = true;
    left = Math.min(left, r.left);
    right = Math.max(right, r.right);
    top = Math.min(top, r.top);
    bottom = Math.max(bottom, r.bottom);
  }
  if (!any) {
    const br = bar.getBoundingClientRect();
    left = br.left;
    right = br.right;
    top = br.top;
    bottom = br.bottom;
  }
  const pad = 3;
  return (
    clientX >= left - pad &&
    clientX <= right + pad &&
    clientY >= top - pad &&
    clientY <= bottom + pad
  );
}

/** Resting (unscaled) centers relative to bar content left — avoids layout feedback. */
function restingCenters(items: DockItem[], padX: number): Map<string, number> {
  const map = new Map<string, number>();
  let x = padX;
  items.forEach((item, i) => {
    if (i > 0) x += ICON_GAP;
    if (item.kind === "separator") {
      x += SEP_LAYOUT_W;
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
    w += item.kind === "separator" ? SEP_LAYOUT_W : ICON_SLOT;
  });
  return Math.max(120, w);
}

function isEphemeralDockId(id: string): boolean {
  return id.startsWith("running:") || id === "running-sep";
}

/** Pinned tile immediately left of the pointer gap (between two icons). */
function resolveAfterItemIdAtClientX(clientX: number): string | null {
  const pins = Array.from(document.querySelectorAll<HTMLElement>("[data-dock-id]"))
    .map((el) => {
      const id = el.dataset.dockId || "";
      if (!id || isEphemeralDockId(id)) return null;
      const r = el.getBoundingClientRect();
      if (r.width < 1) return null;
      return { id, left: r.left, right: r.right, mid: r.left + r.width / 2 };
    })
    .filter((x): x is { id: string; left: number; right: number; mid: number } => !!x)
    .sort((a, b) => a.left - b.left);
  if (!pins.length) return null;

  for (let i = 0; i < pins.length; i++) {
    const cur = pins[i];
    const next = pins[i + 1];
    // Inside a tile: left half → previous; right half → this tile.
    if (clientX >= cur.left && clientX < cur.right) {
      if (clientX < cur.mid) {
        return i > 0 ? pins[i - 1].id : cur.id;
      }
      return cur.id;
    }
    // In the gap before the next tile.
    if (next && clientX >= cur.right && clientX < next.left) {
      return cur.id;
    }
  }
  // Past the last pin (but never treat trash as “after” — caller inserts before it).
  const last = pins[pins.length - 1];
  return clientX >= last.mid ? last.id : pins[0]?.id ?? null;
}

function reorderDockItems(items: DockItem[], from: number, to: number): DockItem[] {
  if (from === to || from < 0 || to < 0 || from >= items.length || to >= items.length) {
    return items;
  }
  const next = items.slice();
  const [it] = next.splice(from, 1);
  next.splice(to, 0, it);
  return next;
}

/** Keep pins out of the ephemeral running:* zone (right side). */
function clampPinnedDestIndex(items: DockItem[], from: number, dest: number): number {
  const without = items.filter((_, i) => i !== from);
  const firstEph = without.findIndex((i) => isEphemeralDockId(i.id));
  const max = firstEph >= 0 ? firstEph : without.length;
  return Math.max(0, Math.min(dest, max));
}

/** Prefs pin order = visible non-running tiles L→R, then any overflow-hidden pins. */
function pinnedOrderIds(display: DockItem[], prefsItems: DockItem[]): string[] {
  const visible = display.filter((d) => !isEphemeralDockId(d.id)).map((d) => d.id);
  const seen = new Set(visible);
  const rest = prefsItems.map((i) => i.id).filter((id) => !seen.has(id));
  return [...visible, ...rest];
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

  // Anchor for “添加分割线”: clicked tile, else the pin left of the pointer gap.
  const afterItemId = (itemId?.trim() || resolveAfterItemIdAtClientX(clientX) || "").trim() || null;

  // Hold AutoHide + skip FE collapse while the menu is open (pointer leaves chrome).
  dockStatusMenuOpen = true;
  await invoke("dock_set_interaction_hold", { hold: true }).catch(() => undefined);

  const visible = await invoke<boolean>("is_status_menu_popup_open");
  if (visible) {
    await invoke("close_status_menu_popup");
    dockStatusMenuOpen = true;
    // close clears hold — re-assert before reopen.
    await invoke("dock_set_interaction_hold", { hold: true }).catch(() => undefined);
  }
  await invoke("open_status_menu_popup", {
    x,
    y,
    fromDock: true,
    itemId: itemId?.trim() || null,
    afterItemId,
    pinBottom,
  });
}

function DockRasterGlyph({ src }: { src: string }) {
  return <img className="dock-icon" src={src} alt="" draggable={false} />;
}

/** Default draw scale when prefs omit iconScale (matches Rust default_icon_scale). */
const DEFAULT_ICON_SCALE = 0.9;

function useAutoPlateColor(pngBase64: string | null | undefined): string | null {
  const key = (pngBase64 || "").trim() || null;
  const [color, setColor] = useState<string | null>(() =>
    key ? peekCachedPlateColor(key) : null,
  );
  useEffect(() => {
    if (!key) {
      setColor(null);
      return;
    }
    const cached = peekCachedPlateColor(key);
    if (cached) {
      setColor(cached);
      return;
    }
    let alive = true;
    void plateColorFromPngBase64(key).then((c) => {
      if (alive) setColor(c);
    });
    return () => {
      alive = false;
    };
  }, [key]);
  return color;
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
  const iconScale =
    typeof item.iconScale === "number" && item.iconScale > 0
      ? item.iconScale
      : DEFAULT_ICON_SCALE;
  const ox = item.iconOffsetX ?? 0;
  const oy = item.iconOffsetY ?? 0;
  const bgRaw = (item.iconBg ?? "").trim();
  const src = item.iconPng ? `data:image/png;base64,${item.iconPng}` : null;
  const autoPlate = useDockIconPlate(src);
  const autoColor = useAutoPlateColor(bgRaw ? null : item.iconPng);

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
  } else if (src && autoPlate) {
    // Transparent-edge icons: plate = island-notify style dominant color.
    plateClass += " has-custom-bg";
    plateBg = autoColor || DOCK_AUTO_PLATE_BG;
  } else if (src) {
    plateClass += " has-bg";
  } else {
    plateClass += " needs-plate";
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
  /** External file drop hover (pin apps). */
  const [dropHover, setDropHover] = useState(false);
  /** `@hello-pangea/dnd` drag in progress — fan magnification frozen. */
  const [draggingId, setDraggingId] = useState<string | null>(null);
  /** True while pointer is high enough to unpin on drop-outside. */
  const [dragRemoveArmed, setDragRemoveArmed] = useState(false);
  /** Pointer X relative to `.dock-bar` content box. */
  const [localX, setLocalX] = useState<number | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  const suppressClickRef = useRef(false);
  const dndActiveRef = useRef(false);
  const dragRemoveArmedRef = useRef(false);
  const dragStartYRef = useRef(0);
  /** Last pointer during dnd (remove-arm + drop sample). */
  const lastDndPointerRef = useRef({ x: 0, y: 0 });
  /**
   * Blocks fan only for the drop's own synchronous pointer samples.
   * Cleared on microtask — next real move applies magnify immediately.
   */
  const postDndFanBlockedRef = useRef(false);
  /** Skip prefs/display churn while a quiet reorder persist is in flight. */
  const persistQuietRef = useRef(false);
  const displayItemsRef = useRef<DockItem[]>([]);
  const prefsRef = useRef<DockPrefs | null>(null);
  const rafRef = useRef(0);
  /** Coalesce fan X to one setState per frame. */
  const fanMoveRafRef = useRef(0);
  const pendingFanClientXRef = useRef<number | null>(null);
  const expandedRef = useRef(false);
  const expandInflightRef = useRef<boolean | null>(null);
  /** Fan only after HWND widen succeeds — never grow icons on resting width. */
  const fanArmedRef = useRef(false);
  const [fanArmed, setFanArmed] = useState(false);
  /** Extend stack hit height through headroom while hovering. */
  const [fanLive, setFanLive] = useState(false);
  /** Chrome stroke width tracks expand tween (not fanLive — that snapped early). */
  const [barWide, setBarWide] = useState(false);
  /** Hold chrome/glass wide while finishing unmagnify → then shrink (no icon leak). */
  const [fanCollapsing, setFanCollapsing] = useState(false);
  const widthTweenTimerRef = useRef(null as number | null);
  /** Ignore stale dock_set_hover_expand responses after rapid enter/leave. */
  const expandSeqRef = useRef(0);
  /** True while pointer is inside the dock bar (gates fan arm). */
  const pointerInsideRef = useRef(false);
  /** Last pointer client coords — fan X while hovering / widen arm. */
  const lastPointerClientRef = useRef<{ x: number; y: number } | null>(null);
  /** Ignore flaky pointerleave while dock HWND slides up under the cursor. */
  const showSettleUntilRef = useRef(0);
  const showResumeTimersRef = useRef<number[]>([]);
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

  const cancelWidthTweenTimer = () => {
    if (widthTweenTimerRef.current != null) {
      window.clearTimeout(widthTweenTimerRef.current);
      widthTweenTimerRef.current = null;
    }
  };

  const cancelShowResumeTimers = () => {
    for (const id of showResumeTimersRef.current) {
      window.clearTimeout(id);
    }
    showResumeTimersRef.current = [];
  };

  const disarmFan = () => {
    fanArmGenRef.current += 1;
    fanArmedRef.current = false;
    pendingFanXRef.current = null;
    setFanArmed(false);
    setLocalX(null);
  };

  /** Commit rest scales immediately (hide slide must not paint magnified tiles). */
  const snapFanToRestSync = () => {
    fanArmGenRef.current += 1;
    fanArmedRef.current = false;
    pendingFanXRef.current = null;
    if (fanMoveRafRef.current) {
      cancelAnimationFrame(fanMoveRafRef.current);
      fanMoveRafRef.current = 0;
    }
    pendingFanClientXRef.current = null;
    flushSync(() => {
      setFanCollapsing(true);
      setFanArmed(false);
      setLocalX(null);
      setFanLive(false);
      setBarWide(false);
    });
  };

  const queueFanFromClientX = (clientX: number) => {
    pendingFanClientXRef.current = clientX;
    if (fanMoveRafRef.current) return;
    fanMoveRafRef.current = requestAnimationFrame(() => {
      fanMoveRafRef.current = 0;
      const cx = pendingFanClientXRef.current;
      const bar = barRef.current;
      if (cx == null || !bar) return;
      const rect = bar.getBoundingClientRect();
      const pad = dockPadX(prefsRef.current?.cornerRadiusPx ?? 20);
      const restW = restingBarWidth(displayItemsRef.current, pad);
      // Bar is full HWND width; icons are centered — map into resting content space.
      const offset = (rect.width - restW) / 2;
      const x = cx - rect.left - offset;
      // Buffer pointer until widen completes — never drive scales early.
      if (!fanArmedRef.current || !expandedRef.current) {
        pendingFanXRef.current = x;
        return;
      }
      setLocalX(x);
    });
  };

  /** After AutoHide reveal, HWND slides under a stationary cursor — synthesize hover. */
  const resumeHoverAfterShow = async () => {
    if (dndActiveRef.current || postDndFanBlockedRef.current) return;
    if (dockStatusMenuOpen) return;
    // Always sample the live OS cursor. Preferring lastPointerClientRef after hide→show
    // (Alt+Tab out of fullscreen, Default mode) reused stale icon-zone coords and
    // falsely armed magnification while the pointer was nowhere near the dock.
    let clientX: number;
    let clientY: number;
    try {
      const pt = await invoke<[number, number] | null>("dock_pointer_client_xy");
      if (!pt) return;
      clientX = pt[0];
      clientY = pt[1];
      lastPointerClientRef.current = { x: clientX, y: clientY };
    } catch {
      return;
    }
    const bar = barRef.current;
    if (!bar) return;
    // Require the icon/sep hit union (X+Y) — bar-wide X alone matches cursors above the
    // chrome when the HWND is already shown for non-hover reasons.
    if (!pointerInFanIconZone(clientX, clientY, bar)) return;
    pointerInsideRef.current = true;
    cancelCollapseTimer();
    if (fanCollapsing) setFanCollapsing(false);
    setFanLive(true);
    queueFanFromClientX(clientX);
    if (!expandedRef.current && expandInflightRef.current !== true) {
      setExpanded(true);
    } else if (expandedRef.current && expandInflightRef.current == null) {
      if (!fanArmedRef.current) {
        fanArmedRef.current = true;
        setFanArmed(true);
      }
      queueFanFromClientX(clientX);
    }
  };

  /** Only after widen has started + a short settle — overlap with glass tween. */
  const armFanAfterWidenPaint = () => {
    const gen = fanArmGenRef.current;
    requestAnimationFrame(() => {
      if (gen !== fanArmGenRef.current) return;
      if (!expandedRef.current) return;
      if (expandInflightRef.current === false) return;
      if (!pointerInsideRef.current) {
        void resumeHoverAfterShow();
        return;
      }
      fanArmedRef.current = true;
      setFanArmed(true);
      const pending = pendingFanXRef.current;
      if (pending != null) {
        setLocalX(pending);
      } else if (lastPointerClientRef.current) {
        queueFanFromClientX(lastPointerClientRef.current.x);
      }
    });
  };

  const setExpanded = (next: boolean) => {
    const seq = ++expandSeqRef.current;
    // Same target: still re-assert backend (heals stuck capsule) but do not nest timers.
    if (expandedRef.current === next && expandInflightRef.current == null) {
      if (next) {
        void invoke<boolean>("dock_set_hover_expand", { expanded: true })
          .then((ok) => {
            if (seq !== expandSeqRef.current) return;
            if (!ok || fanArmedRef.current) return;
            if (pointerInsideRef.current) armFanAfterWidenPaint();
            else void resumeHoverAfterShow();
          })
          .catch(() => undefined);
      }
      return;
    }
    if (expandInflightRef.current === next) return;
    expandInflightRef.current = next;
    cancelWidthTweenTimer();
    if (next) {
      // Phase 1: start widen — fan arms shortly after (overlaps glass tween).
      disarmFan();
    } else {
      // Width collapse only — fan must already be cleared by beginCollapseAfterFanRest.
      setFanLive(false);
    }
    // Start glass first; setBarWide in `.then` so chrome CSS begins with Composition.
    void invoke<boolean>("dock_set_hover_expand", { expanded: next })
      .then((ok) => {
        if (seq !== expandSeqRef.current) {
          if (expandInflightRef.current === next) {
            expandInflightRef.current = null;
          }
          // Leave cancelled this expand — don't leave BE stuck wide with FE at rest.
          if (next && !pointerInsideRef.current && !expandedRef.current) {
            setBarWide(false);
            void invoke<boolean>("dock_set_hover_expand", { expanded: false }).catch(
              () => undefined,
            );
          }
          return;
        }
        if (!ok) {
          if (next) {
            expandInflightRef.current = null;
            window.setTimeout(() => {
              if (seq !== expandSeqRef.current) return;
              if (!expandedRef.current && pointerInsideRef.current) setExpanded(true);
            }, 32);
            return;
          }
          setBarWide(false);
          expandedRef.current = false;
          if (expandInflightRef.current === next) {
            expandInflightRef.current = null;
          }
          return;
        }
        expandedRef.current = next;
        // Collapse: barWide false starts chrome shrink together with glass (fan already gone).
        // Expand: barWide true starts chrome grow; fan arms after FAN_ARM_DELAY_MS.
        setBarWide(next);
        if (!next) {
          if (expandInflightRef.current === next) {
            expandInflightRef.current = null;
          }
          return;
        }
        widthTweenTimerRef.current = window.setTimeout(() => {
          widthTweenTimerRef.current = null;
          if (seq !== expandSeqRef.current) {
            if (expandInflightRef.current === next) {
              expandInflightRef.current = null;
            }
            return;
          }
          if (expandInflightRef.current === next) {
            expandInflightRef.current = null;
          }
          if (expandedRef.current) {
            if (pointerInsideRef.current) armFanAfterWidenPaint();
            else void resumeHoverAfterShow();
          }
        }, FAN_ARM_DELAY_MS);
      })
      .catch(() => {
        if (seq !== expandSeqRef.current) return;
        setBarWide(expandedRef.current);
        cancelWidthTweenTimer();
        if (expandInflightRef.current === next) {
          expandInflightRef.current = null;
        }
      });
  };

  const beginCollapseAfterFanRest = () => {
    // Leave / past icon peak: snap magnification immediately (any direction).
    pointerInsideRef.current = false;
    expandSeqRef.current += 1;
    cancelWidthTweenTimer();
    expandInflightRef.current = null;
    cancelCollapseTimer();
    snapFanToRestSync();
    // Next frame: drop collapsing hold and collapse hover width (icons already at rest).
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = null;
      setFanCollapsing(false);
      if (pointerInsideRef.current) return;
      if (expandedRef.current || expandInflightRef.current === true) {
        setExpanded(false);
      } else {
        expandedRef.current = false;
        setBarWide(false);
        void invoke<boolean>("dock_set_hover_expand", { expanded: false }).catch(
          () => undefined,
        );
      }
    }, 0);
  };
  const beginCollapseRef = useRef(beginCollapseAfterFanRest);
  beginCollapseRef.current = beginCollapseAfterFanRest;

  /** While fan/expand is active, watch all pointer moves — bar is HWND-wide so
   *  sliding into side pad never fires pointerleave, but must still snap mag. */
  useEffect(() => {
    if (!fanArmed && !fanLive && !fanCollapsing && !barWide) return;

    const maybeCollapse = (clientX: number, clientY: number) => {
      if (dndActiveRef.current || dockStatusMenuOpen || postDndFanBlockedRef.current) {
        return;
      }
      if (performance.now() < showSettleUntilRef.current) return;
      const bar = barRef.current;
      if (!bar) return;
      lastPointerClientRef.current = { x: clientX, y: clientY };
      if (pointerInFanIconZone(clientX, clientY, bar)) return;
      if (
        fanArmedRef.current ||
        pointerInsideRef.current ||
        (expandedRef.current && collapseTimerRef.current == null)
      ) {
        beginCollapseRef.current();
      }
    };

    const onMove = (e: PointerEvent) => {
      maybeCollapse(e.clientX, e.clientY);
    };
    // Mouse left the webview entirely (OS desktop) — always snap.
    const onDocLeave = () => {
      if (dndActiveRef.current || dockStatusMenuOpen) return;
      if (
        fanArmedRef.current ||
        pointerInsideRef.current ||
        expandedRef.current
      ) {
        beginCollapseRef.current();
      }
    };

    window.addEventListener("pointermove", onMove, true);
    document.documentElement.addEventListener("mouseleave", onDocLeave);
    return () => {
      window.removeEventListener("pointermove", onMove, true);
      document.documentElement.removeEventListener("mouseleave", onDocLeave);
    };
  }, [fanArmed, fanLive, fanCollapsing, barWide]);

  useEffect(() => installChromeHoverTipGlobalDismiss(), []);

  useEffect(() => {
    let unOpen: (() => void) | undefined;
    let unClose: (() => void) | undefined;
    void listen("status-menu-popup-opened", () => {
      dockStatusMenuOpen = true;
      cancelCollapseTimer();
      void invoke("dock_set_interaction_hold", { hold: true }).catch(() => undefined);
    }).then((fn) => {
      unOpen = fn;
    });
    void listen("status-menu-popup-closed", () => {
      dockStatusMenuOpen = false;
      void invoke("dock_set_interaction_hold", { hold: false }).catch(() => undefined);
    }).then((fn) => {
      unClose = fn;
    });
    return () => {
      unOpen?.();
      unClose?.();
    };
  }, []);

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
        // Never mutate Draggable sizes/count mid-drag (hello-pangea/dnd contract).
        if (dndActiveRef.current || persistQuietRef.current) return;
        const items = await invoke<DockItem[]>("get_dock_display_items");
        if (cancelled || dndActiveRef.current) return;
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
      if (cancelled) return;
      // Reorder persist emits stripped pins (no icons) — applying that prefs
      // blob mid-fan hitchs the bar. Ignore until quiet persist finishes.
      if (dndActiveRef.current || persistQuietRef.current) return;
      setPrefs(e.payload);
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
    if (!compactTip || draggingId || dropHover) return;
    const t = window.setTimeout(() => setCompactTip(null), 6000);
    return () => window.clearTimeout(t);
  }, [compactTip, draggingId, dropHover]);

  // Native OLE file-drop (Rust) → tip + prefs refresh.
  useEffect(() => {
    let un: (() => void) | undefined;
    void listen<{ phase?: string; count?: number }>("dock-file-drag", (ev) => {
      const phase = ev.payload?.phase;
      const count = ev.payload?.count ?? 0;
      if (phase === "enter") {
        setDropHover(true);
        const bar = barRef.current;
        const r = bar?.getBoundingClientRect();
        void showChromeHoverTip({
          text: "松开以固定到 Dock",
          x: r ? r.left + r.width / 2 : window.innerWidth / 2,
          y: r ? r.top : 8,
          placement: "above",
          gap: 10,
          immediate: true,
        });
      } else if (phase === "leave") {
        setDropHover(false);
        void hideChromeHoverTip();
      } else if (phase === "drop") {
        setDropHover(false);
        if (count > 0) {
          flashChromeTip(`已固定 ${count} 个到 Dock`);
        } else {
          void hideChromeHoverTip();
        }
      } else if (phase === "error") {
        setDropHover(false);
        flashChromeTip("无法固定到 Dock（请拖入 .exe / .lnk）");
      }
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, []);

  displayItemsRef.current = displayItems;
  prefsRef.current = prefs;

  // Track remove-arm + last pointer while dragging — tip is shown once on drag start only.
  useEffect(() => {
    if (!draggingId) return;
    const onMove = (e: PointerEvent) => {
      lastDndPointerRef.current = { x: e.clientX, y: e.clientY };
      const chrome = document.querySelector(".dock-chrome") as HTMLElement | null;
      const top = chrome?.getBoundingClientRect().top ?? window.innerHeight - 52;
      const item = displayItemsRef.current.find((it) => it.id === draggingId);
      const canRemove =
        !!item && item.kind === "app" && !item.id.startsWith("running:");
      const outside = e.clientY < top - 8 || dragStartYRef.current - e.clientY >= 36;
      const armed = canRemove && outside;
      if (armed === dragRemoveArmedRef.current) return;
      dragRemoveArmedRef.current = armed;
      setDragRemoveArmed(armed);
    };
    window.addEventListener("pointermove", onMove);
    return () => window.removeEventListener("pointermove", onMove);
  }, [draggingId]);

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
  const cornerRadius = prefs?.cornerRadiusPx ?? 20;
  /** Match Composition capsule: rest = content; hover = content + FAN_EXTRA (not 100%). */
  const chromeWide = barWide || fanCollapsing || !!draggingId;
  const chromeRestPx = restingBarWidth(displayItems, padX);
  const chromeWidthPx = chromeWide ? chromeRestPx + DOCK_FAN_EXTRA : chromeRestPx;
  const centers = useMemo(
    () => restingCenters(displayItems, padX),
    [displayItems, padX],
  );

  const scales = useMemo(() => {
    const map = new Map<string, number>();
    // During dnd, keep resting widths so hello-pangea/dnd's dimension model stays valid.
    // Hover fan is unchanged whenever not dragging.
    if (draggingId || !fanArmed || localX == null) return map;
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
  }, [displayItems, localX, maxScale, centers, fanArmed, draggingId]);

  /** Bumped on AutoHide hide so icon DOM remounts — clears frozen mid-tween sizes. */
  const [iconMountGen, setIconMountGen] = useState(0);

  // AutoHide show/hide — clear fan on hide; on show resume hover under stationary cursor.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    void listen<{ visible?: boolean }>("dock-visibility", (ev) => {
      if (ev.payload?.visible === false) {
        if (rafRef.current) cancelAnimationFrame(rafRef.current);
        cancelCollapseTimer();
        cancelWidthTweenTimer();
        cancelShowResumeTimers();
        expandSeqRef.current += 1;
        expandInflightRef.current = null;
        pointerInsideRef.current = false;
        // Drop cached pointer so the next show cannot revive fan from pre-hide hover.
        lastPointerClientRef.current = null;
        showSettleUntilRef.current = 0;
        expandedRef.current = false;
        snapFanToRestSync();
        setIconMountGen((n) => n + 1);
        requestAnimationFrame(() => {
          setFanCollapsing(false);
        });
        void invoke<boolean>("dock_set_hover_expand", { expanded: false }).catch(
          () => undefined,
        );
        return;
      }
      if (ev.payload?.visible === true) {
        // Match backend settle — ignore leave while HWND slides under cursor.
        showSettleUntilRef.current = performance.now() + 700;
        cancelShowResumeTimers();
        // Probe early + a couple retries (slide still moving).
        const delays = [0, 50, 140, 280];
        showResumeTimersRef.current = delays.map((ms) =>
          window.setTimeout(() => {
            void resumeHoverAfterShow();
          }, ms),
        );
      }
    }).then((u) => {
      unsub = u;
    });
    return () => {
      unsub?.();
      cancelShowResumeTimers();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount-only listener; snap uses refs
  }, []);

  const onBarPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!prefs || dndActiveRef.current) return;
    if (postDndFanBlockedRef.current) return;
    lastPointerClientRef.current = { x: e.clientX, y: e.clientY };
    const bar = barRef.current;
    // Past magnified icon peak / bar sides → cancel mag immediately (any direction).
    if (bar && !pointerInFanIconZone(e.clientX, e.clientY, bar)) {
      if (
        fanArmedRef.current ||
        pointerInsideRef.current ||
        (expandedRef.current && collapseTimerRef.current == null && !fanCollapsing)
      ) {
        beginCollapseAfterFanRest();
      }
      return;
    }
    pointerInsideRef.current = true;
    cancelCollapseTimer();
    if (fanCollapsing) setFanCollapsing(false);
    if (!fanLive) setFanLive(true);
    // Widen at most once per hover session; moves only update fan X.
    if (!expandedRef.current && expandInflightRef.current !== true) {
      setExpanded(true);
    } else if (expandedRef.current && !fanArmedRef.current && expandInflightRef.current == null) {
      fanArmedRef.current = true;
      setFanArmed(true);
    }
    queueFanFromClientX(e.clientX);
  };

  const onBarPointerEnter = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (dndActiveRef.current || postDndFanBlockedRef.current) return;
    lastPointerClientRef.current = { x: e.clientX, y: e.clientY };
    pointerInsideRef.current = true;
    cancelCollapseTimer();
    if (fanCollapsing) setFanCollapsing(false);
    setFanLive(true);
    // From default width only when this session is not already wide.
    if (!expandedRef.current && expandInflightRef.current !== true) {
      setExpanded(true);
    } else if (expandedRef.current && !fanArmedRef.current && expandInflightRef.current == null) {
      fanArmedRef.current = true;
      setFanArmed(true);
    }
    queueFanFromClientX(e.clientX);
  };

  const onBarPointerLeave = () => {
    if (dndActiveRef.current || dockStatusMenuOpen) return;
    // AutoHide slide-up under cursor often fires a spurious leave — ignore during settle.
    if (performance.now() < showSettleUntilRef.current) {
      void resumeHoverAfterShow();
      return;
    }
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    if (fanMoveRafRef.current) {
      cancelAnimationFrame(fanMoveRafRef.current);
      fanMoveRafRef.current = 0;
    }
    pendingFanClientXRef.current = null;
    beginCollapseAfterFanRest();
  };

  const onBarPointerCancel = () => {
    if (dndActiveRef.current || dockStatusMenuOpen) return;
    if (performance.now() < showSettleUntilRef.current) {
      void resumeHoverAfterShow();
      return;
    }
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    if (fanMoveRafRef.current) {
      cancelAnimationFrame(fanMoveRafRef.current);
      fanMoveRafRef.current = 0;
    }
    pendingFanClientXRef.current = null;
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

  function canReorder(item: DockItem): boolean {
    // User separators are freely reorderable (Ctrl-only was unreliable: dock
    // webview often misses keydown when unfocused). Ephemeral running-sep stays locked.
    return !isEphemeralDockId(item.id);
  }

  function canDragUnpin(item: DockItem): boolean {
    return item.kind === "app" && !item.id.startsWith("running:");
  }

  function chromeTopY(): number {
    const chrome = document.querySelector(".dock-chrome") as HTMLElement | null;
    return chrome?.getBoundingClientRect().top ?? window.innerHeight - 52;
  }

  /** Status toast above the bar — same chrome tip surface as hover labels. */
  function flashChromeTip(text: string, clientX?: number, clientY?: number) {
    const msg = text.trim();
    if (!msg) return;
    const bar = barRef.current?.getBoundingClientRect();
    const x =
      typeof clientX === "number"
        ? clientX
        : bar
          ? bar.left + bar.width / 2
          : window.innerWidth / 2;
    const y = typeof clientY === "number" ? clientY : bar ? bar.top : chromeTopY();
    void showChromeHoverTip({
      text: msg,
      x,
      y,
      placement: "above",
      gap: 10,
      immediate: true,
    });
    window.setTimeout(() => {
      void hideChromeHoverTip();
    }, 2200);
  }

  function endDndChrome() {
    dndActiveRef.current = false;
    dragRemoveArmedRef.current = false;
    // Ignore the drop frame's pointer sample; microtask clears for the next move.
    postDndFanBlockedRef.current = true;
    queueMicrotask(() => {
      postDndFanBlockedRef.current = false;
    });
    pendingFanXRef.current = null;
    pendingFanClientXRef.current = null;
    if (fanMoveRafRef.current) {
      cancelAnimationFrame(fanMoveRafRef.current);
      fanMoveRafRef.current = 0;
    }
    if (expandedRef.current) {
      fanArmedRef.current = true;
    }
    // Keep this frame light — tip / hold IPC deferred off the drop hitch.
    setDraggingId(null);
    setDragRemoveArmed(false);
    setLocalX(null);
    setFanArmed((v) => (expandedRef.current ? true : v));
    requestAnimationFrame(() => {
      void hideChromeHoverTip();
      void invoke("dock_set_interaction_hold", { hold: false }).catch(() => undefined);
    });
  }

  /** Freeze fan *before* dimension capture so resting widths match the virtual model. */
  function onBeforeCapture(before: BeforeCapture) {
    dndActiveRef.current = true;
    cancelCollapseTimer();
    void hideChromeHoverTip();
    void invoke("dock_set_interaction_hold", { hold: true }).catch(() => undefined);
    // Must commit resting slot widths before rfd measures the DOM.
    flushSync(() => {
      setFanLive(true);
      disarmFan();
      setDraggingId(before.draggableId);
      setDragRemoveArmed(false);
    });
  }

  function onDragStart(start: DragStart) {
    dndActiveRef.current = true;
    void invoke("dock_set_interaction_hold", { hold: true }).catch(() => undefined);
    setDraggingId(start.draggableId);
    setDragRemoveArmed(false);
    dragRemoveArmedRef.current = false;
    dragStartYRef.current = 0;
    // Capture start Y on next pointer sample (library owns the gesture).
    const once = (e: PointerEvent) => {
      dragStartYRef.current = e.clientY;
      lastDndPointerRef.current = { x: e.clientX, y: e.clientY };
      window.removeEventListener("pointermove", once);
    };
    window.addEventListener("pointermove", once, { once: true });
    suppressClickRef.current = true;
    const item = displayItemsRef.current.find((it) => it.id === start.draggableId);
    const tip =
      item && canDragUnpin(item) ? "横向调整位置，拖出可移除" : "拖动调整位置";
    const bar = barRef.current?.getBoundingClientRect();
    void showChromeHoverTip({
      text: tip,
      x: bar ? bar.left + bar.width / 2 : window.innerWidth / 2,
      y: bar ? bar.top : chromeTopY(),
      placement: "above",
      gap: 10,
      immediate: true,
    });
  }

  function onDragUpdate(update: DragUpdate) {
    if (!update.destination) {
      const item = displayItemsRef.current.find((it) => it.id === update.draggableId);
      if (item && canDragUnpin(item) && dragRemoveArmedRef.current) {
        setDragRemoveArmed(true);
      }
    }
  }

  function onDragEnd(result: DropResult) {
    const dragId = result.draggableId;
    const item = displayItemsRef.current.find((it) => it.id === dragId) ?? null;
    const armed = dragRemoveArmedRef.current;
    endDndChrome();
    window.setTimeout(() => {
      suppressClickRef.current = false;
    }, 0);

    // Drop outside + armed upward → unpin (apps only).
    if (!result.destination) {
      if (result.reason === "DROP" && item && canDragUnpin(item) && armed) {
        void invoke<DockPrefs>("dock_unpin_item", { itemId: item.id })
          .then((next) => {
            setPrefs(next);
            setDisplayItems(next.items);
            flashChromeTip("已从 Dock 移除", undefined, chromeTopY());
            void invoke<DockItem[]>("get_dock_display_items")
              .then((items) => {
                if (!dndActiveRef.current) setDisplayItems(items);
              })
              .catch(() => undefined);
          })
          .catch((err) => {
            console.error(err);
            flashChromeTip(String(err), undefined, chromeTopY());
          });
      }
      return;
    }

    const from = result.source.index;
    const to = clampPinnedDestIndex(displayItemsRef.current, from, result.destination.index);
    if (from === to) return;
    // Ctrl may be released before mouse-up — still persist a separator move.
    if (item && isEphemeralDockId(item.id)) return;

    const nextDisplay = reorderDockItems(displayItemsRef.current, from, to);
    displayItemsRef.current = nextDisplay;
    layoutSigRef.current = nextDisplay.map((i) => `${i.id}:${i.kind}`).join("|");
    setDisplayItems(nextDisplay);
    const p = prefsRef.current;
    const ordered = pinnedOrderIds(nextDisplay, p?.items ?? []);
    // Persist off the drop frame; never apply stripped prefs (wipes icons / hitch).
    persistQuietRef.current = true;
    window.requestAnimationFrame(() => {
      void invoke<DockPrefs>("dock_reorder_items", { orderedIds: ordered })
        .then(() => {
          const cur = prefsRef.current;
          if (!cur) return;
          const byId = new Map(cur.items.map((it) => [it.id, it]));
          const items = ordered
            .map((id) => byId.get(id))
            .filter((it): it is DockItem => !!it);
          for (const it of cur.items) {
            if (!items.some((x) => x.id === it.id)) items.push(it);
          }
          const merged = { ...cur, items };
          prefsRef.current = merged;
          setPrefs(merged);
        })
        .catch((err) => {
          console.error(err);
          flashChromeTip(String(err), undefined, chromeTopY());
        })
        .finally(() => {
          persistQuietRef.current = false;
        });
    });
  }

  if (!prefs) {
    return <div className="dock-shell dock-loading" />;
  }

  return (
    <DragDropContext
      onBeforeCapture={onBeforeCapture}
      onDragStart={onDragStart}
      onDragUpdate={onDragUpdate}
      onDragEnd={onDragEnd}
    >
      <div
        className={`dock-shell${fanLive ? " is-fan-live" : ""}${fanCollapsing ? " is-fan-collapsing" : ""}${chromeWide ? " is-bar-wide" : ""}${dropHover ? " is-drop-hover" : ""}${draggingId ? " is-dragging-item is-dnd-active" : ""}${dragRemoveArmed ? " is-dnd-remove" : ""}`}
        data-mode={prefs.displayMode}
        data-mag={magOn ? "on" : "off"}
        onContextMenu={onBackgroundContextMenu}
        onDragEnter={(e: ReactDragEvent) => {
          e.preventDefault();
          setDropHover(true);
        }}
        onDragOver={(e: ReactDragEvent) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
          if (!dropHover) setDropHover(true);
        }}
        onDragLeave={(e: ReactDragEvent) => {
          const related = e.relatedTarget as Node | null;
          if (related && e.currentTarget.contains(related)) return;
          setDropHover(false);
        }}
        onDrop={(e: ReactDragEvent) => {
          e.preventDefault();
          setDropHover(false);
        }}
      >
        {compactTip && !draggingId && !dropHover ? (
          <div className="dock-compact-tip" role="status">
            {compactTip}
          </div>
        ) : null}
        <div className="dock-stack">
          <div
            className="dock-chrome"
            aria-hidden
            style={
              {
                ["--dock-radius" as string]: `${cornerRadius}px`,
                width: chromeWidthPx,
              } as CSSProperties
            }
          />
          <Droppable droppableId="dock-bar" direction="horizontal">
            {(dropProvided) => (
              <div
                ref={(el) => {
                  barRef.current = el;
                  dropProvided.innerRef(el);
                }}
                {...dropProvided.droppableProps}
                className="dock-bar"
                style={{ paddingLeft: padX, paddingRight: padX }}
                onPointerEnter={onBarPointerEnter}
                onPointerMove={onBarPointerMove}
                onPointerLeave={onBarPointerLeave}
                onPointerCancel={onBarPointerCancel}
              >
                {displayItems.map((item, index) => {
                  if (item.kind === "separator") {
                    const ephemeral = isEphemeralDockId(item.id);
                    const sepMovable = !ephemeral;
                    return (
                      <Draggable
                        key={item.id}
                        draggableId={item.id}
                        index={index}
                        isDragDisabled={!sepMovable}
                        disableInteractiveElementBlocking
                      >
                        {(dragProvided, snapshot) => (
                          <div
                            ref={dragProvided.innerRef}
                            {...dragProvided.draggableProps}
                            {...dragProvided.dragHandleProps}
                            data-dock-id={item.id}
                            className={`dock-sep${ephemeral ? " is-ephemeral" : ""}${sepMovable ? " is-sep-movable" : ""}${snapshot.isDragging ? " is-dragging" : ""}`}
                            title={ephemeral ? undefined : "拖动调整分割线 · 右键删除"}
                            style={dragProvided.draggableProps.style}
                            onContextMenu={(e) => {
                              if (ephemeral) return;
                              e.preventDefault();
                              e.stopPropagation();
                              void hideChromeHoverTip();
                              void openStatusMenuAtClientPoint(
                                e.clientX,
                                e.clientY,
                                item.id,
                              ).catch(console.error);
                            }}
                          />
                        )}
                      </Draggable>
                    );
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
                  const reorderable = canReorder(item);
                  return (
                    <Draggable
                      key={`${item.id}#${iconMountGen}`}
                      draggableId={item.id}
                      index={index}
                      isDragDisabled={!reorderable}
                      // Root used to be <button>; rfd blocks drag on interactive tags.
                      disableInteractiveElementBlocking
                    >
                      {(dragProvided, snapshot) => (
                        <div
                          role="button"
                          tabIndex={0}
                          data-dock-id={item.id}
                          ref={dragProvided.innerRef}
                          {...dragProvided.draggableProps}
                          {...dragProvided.dragHandleProps}
                          className={`dock-item${running ? " is-running" : ""}${scale > 1.02 ? " is-magnified" : ""}${snapshot.isDragging ? " is-dragging" : ""}${reorderable ? " is-unpinable" : ""}`}
                          {...(draggingId ? {} : dockIconTipPointerProps(label, { gap: 8 }))}
                          style={
                            {
                              ...dragProvided.draggableProps.style,
                              ["--dock-scale" as string]: String(scale),
                              ["--dock-slot" as string]: `${slot}px`,
                              ["--dock-hit" as string]: `${slot}px`,
                            } as CSSProperties
                          }
                          onClick={() => {
                            if (suppressClickRef.current) return;
                            void hideChromeHoverTip();
                            void onItemClick(item);
                          }}
                          onKeyDown={(e) => {
                            if (e.key !== "Enter" && e.key !== " ") return;
                            e.preventDefault();
                            if (suppressClickRef.current) return;
                            void hideChromeHoverTip();
                            void onItemClick(item);
                          }}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            void hideChromeHoverTip();
                            void openStatusMenuAtClientPoint(
                              e.clientX,
                              e.clientY,
                              item.id,
                            ).catch(console.error);
                          }}
                        >
                          <span className="dock-hit">
                            <DockItemGlyph item={item} label={label} scale={scale} />
                          </span>
                          <span
                            className={`dock-dot${running ? " is-on" : ""}`}
                            aria-hidden
                          />
                        </div>
                      )}
                    </Draggable>
                  );
                })}
                {dropProvided.placeholder}
              </div>
            )}
          </Droppable>
        </div>
      </div>
    </DragDropContext>
  );
}
