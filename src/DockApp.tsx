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
  hostTipPointerProps,
  installChromeHoverTipGlobalDismiss,
} from "./chromeHoverTip";
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
};

type DockPrefs = {
  enabled: boolean;
  displayMode: string;
  hideSystemTaskbar: boolean;
  items: DockItem[];
  hotkey: string;
  magnification?: number;
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
const STATUS_MENU_H = 292;
const STATUS_MENU_GAP = 8;
const STATUS_MENU_MARGIN = 8;

/** Icon slot width (matches CSS / Rust DOCK_ICON). */
const ICON_SLOT = 40;
const ICON_GAP = 6;
const BAR_PAD_X = 2;
const SEP_W = 10;
/** How many icon-widths the fan reaches on each side. */
const MAG_RANGE = 2.25;
/** Fixed magnification — not user-configurable (matches Rust DOCK_MAG_SCALE). */
const DOCK_MAG = 1.6;

function fanScale(distancePx: number, maxScale: number): number {
  if (maxScale <= 1.001) return 1;
  const reach = ICON_SLOT * MAG_RANGE;
  if (reach <= 0 || distancePx >= reach) return 1;
  const t = distancePx / reach;
  const w = Math.cos((t * Math.PI) / 2);
  return 1 + (maxScale - 1) * w * w;
}

/** Resting (unscaled) centers relative to bar content left — avoids layout feedback. */
function restingCenters(items: DockItem[]): Map<string, number> {
  const map = new Map<string, number>();
  let x = BAR_PAD_X;
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
function restingBarWidth(items: DockItem[]): number {
  let w = BAR_PAD_X * 2;
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
async function openStatusMenuAtClientPoint(clientX: number, clientY: number) {
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
  // Popup will auto-fit height; pin bottom so it grows upward toward the dock.
  try {
    sessionStorage.setItem("wh.statusMenu.pinBottom", String(originY + clientY - STATUS_MENU_GAP));
  } catch {
    /* noop */
  }
  await invoke("open_status_menu_popup", { x, y });
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
  const layoutSigRef = useRef("");
  const winExeSigRef = useRef("");
  const winRunSigRef = useRef("");

  const setExpanded = (next: boolean) => {
    if (expandedRef.current === next) return;
    if (expandInflightRef.current === next) return;
    expandInflightRef.current = next;
    void invoke<boolean>("dock_set_hover_expand", { expanded: next })
      .then((ok) => {
        if (ok) {
          expandedRef.current = next;
        }
        // If failed (place lock / not shown), clear inflight so the next move retries.
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

  const centers = useMemo(() => restingCenters(displayItems), [displayItems]);

  const scales = useMemo(() => {
    const map = new Map<string, number>();
    if (localX == null) return map;
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
  }, [displayItems, localX, maxScale, centers]);

  // Keep Rust icon-rect cache fresh for OS title-bar minimize → genie.
  useEffect(() => {
    let cancelled = false;
    const report = async () => {
      try {
        const win = getCurrentWindow();
        const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
        if (cancelled) return;
        const rects: Array<{ id: string; x: number; y: number; w: number; h: number }> = [];
        document.querySelectorAll<HTMLElement>("[data-dock-item-id]").forEach((el) => {
          const id = el.dataset.dockItemId;
          if (!id) return;
          const r = el.getBoundingClientRect();
          if (r.width < 2 || r.height < 2) return;
          rects.push({
            id,
            x: outer.x / factor + r.left,
            y: outer.y / factor + r.top,
            w: Math.max(8, r.width),
            h: Math.max(8, r.height),
          });
        });
        if (rects.length) {
          await invoke("dock_report_icon_rects", { rects });
        }
      } catch {
        /* noop */
      }
    };
    void report();
    const t = window.setInterval(() => void report(), 400);
    return () => {
      cancelled = true;
      window.clearInterval(t);
    };
  }, [displayItems, scales]);

  // AutoHide / hide snap must clear fan + collapse HWND pad.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    void listen<{ visible?: boolean }>("dock-visibility", (ev) => {
      if (ev.payload?.visible === false) {
        if (rafRef.current) cancelAnimationFrame(rafRef.current);
        setLocalX(null);
        setExpanded(false);
      }
    }).then((u) => {
      unsub = u;
    });
    return () => unsub?.();
  }, []);

  const onBarPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const bar = barRef.current;
    if (!bar || !prefs) return;
    setExpanded(true);
    const rect = bar.getBoundingClientRect();
    // Map through live (scaled) bar width → resting layout X for stable fan centers.
    const restW = restingBarWidth(displayItems);
    const x = ((e.clientX - rect.left) / Math.max(rect.width, 1)) * restW;
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      setLocalX(x);
    });
  };

  const onBarPointerLeave = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    setLocalX(null);
    setExpanded(false);
  };

  const onBarPointerCancel = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    setLocalX(null);
    setExpanded(false);
  };

  /** Sampled on pointerdown — click steals focus to Dock, so FG must be read before that. */
  const fgIntentRef = useRef<Record<string, Promise<boolean>>>({});

  async function iconScreenRect(el: HTMLElement): Promise<{ x: number; y: number; w: number; h: number }> {
    const win = getCurrentWindow();
    const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    const r = el.getBoundingClientRect();
    return {
      x: outer.x / factor + r.left,
      y: outer.y / factor + r.top,
      w: Math.max(8, r.width),
      h: Math.max(8, r.height),
    };
  }

  async function onItemClick(item: DockItem, el: HTMLElement | null) {
    if (item.kind === "separator") return;
    setLocalX(null);
    setExpanded(false);
    try {
      const icon = el
        ? await iconScreenRect(el)
        : { x: 0, y: 0, w: ICON_SLOT, h: ICON_SLOT };

      if (item.kind === "app") {
        const parked = await invoke<boolean>("genie_is_parked", { itemId: item.id });
        if (parked) {
          await invoke("genie_restore_app", { itemId: item.id, icon });
          return;
        }
        // Prefer pointerdown sample; fall back to live check (usually false after focus steal).
        const wasFg = await (fgIntentRef.current[item.id] ??
          invoke<boolean>("genie_arm_minimize_intent", { itemId: item.id }).catch(() => false));
        console.info("[DockApp] genie intent", item.id, "wasFg=", wasFg);
        if (wasFg) {
          try {
            await invoke("genie_minimize_app", { itemId: item.id, icon });
            return;
          } catch (err) {
            console.error("[DockApp] genie minimize failed", err);
          }
        }
      }

      // Do NOT steal focus back to Dock after launch — that breaks genie FG tracking.
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
      className="dock-shell"
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
          onPointerEnter={() => setExpanded(true)}
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
                data-dock-item-id={item.kind === "app" ? item.id : undefined}
                {...hostTipPointerProps(label)}
                style={style}
                onPointerDown={() => {
                  if (item.kind !== "app") return;
                  // Must sample before click steals foreground to the Dock.
                  fgIntentRef.current[item.id] = invoke<boolean>("genie_arm_minimize_intent", {
                    itemId: item.id,
                  }).catch(() => false);
                }}
                onClick={(e) => {
                  void hideChromeHoverTip();
                  void onItemClick(item, e.currentTarget);
                }}
                onContextMenu={(e) => e.preventDefault()}
              >
                <span className="dock-hit">
                  {item.iconPng ? (
                    <img
                      className="dock-icon"
                      src={`data:image/png;base64,${item.iconPng}`}
                      alt=""
                      draggable={false}
                    />
                  ) : (
                    <span className="dock-icon-fallback" aria-hidden>
                      {(label || "?").charAt(0)}
                    </span>
                  )}
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
