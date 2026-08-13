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
};

type HubWindow = {
  id: string;
  hwnd: number;
  title: string;
  exe?: string | null;
  exeName?: string | null;
};

const STATUS_MENU_W = 200;
const STATUS_MENU_H = 248;
const STATUS_MENU_GAP = 8;
const STATUS_MENU_MARGIN = 8;

/** Icon slot width (matches CSS / Rust DOCK_ICON). */
const ICON_SLOT = 40;
const ICON_GAP = 6;
const BAR_PAD_X = 6;
const SEP_W = 10;
/** How many icon-widths the fan reaches on each side. */
const MAG_RANGE = 2.25;

function clampMagnification(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return 1.6;
  return Math.min(2.5, Math.max(1, n));
}

/** Cosine falloff: focus largest, left/right symmetric to 1.0. Uniform scale only. */
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
  await invoke("open_status_menu_popup", { x, y });
}

export default function DockApp() {
  const [prefs, setPrefs] = useState<DockPrefs | null>(null);
  const [windows, setWindows] = useState<HubWindow[]>([]);
  /** Pointer X relative to `.dock-bar` content box. */
  const [localX, setLocalX] = useState<number | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef(0);

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

    void (async () => {
      try {
        const p = await invoke<DockPrefs>("get_dock_prefs");
        if (!cancelled) setPrefs(p);
      } catch (e) {
        console.error(e);
      }
      try {
        const list = await invoke<HubWindow[]>("list_open_windows");
        if (!cancelled) setWindows(list);
      } catch {
        /* noop */
      }
    })();

    const unsubs: Array<() => void> = [];
    void listen<DockPrefs>("dock-prefs", (e) => {
      if (!cancelled) setPrefs(e.payload);
    }).then((u) => unsubs.push(u));
    void listen<{ windows: HubWindow[] }>("hub-windows-changed", (e) => {
      if (!cancelled) setWindows(e.payload?.windows ?? []);
    }).then((u) => unsubs.push(u));
    void listen("material-prefs", () => {
      void applyMaterial();
    }).then((u) => unsubs.push(u));

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      for (const u of unsubs) u();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  const maxScale = clampMagnification(prefs?.magnification);
  const magOn = maxScale > 1.001;

  const activeIds = useMemo(() => {
    const set = new Set<string>();
    if (!prefs) return set;
    for (const item of prefs.items) {
      if (item.kind !== "app") continue;
      if (windows.some((w) => exeMatches(item, w))) set.add(item.id);
    }
    return set;
  }, [prefs, windows]);

  const centers = useMemo(
    () => (prefs ? restingCenters(prefs.items) : new Map<string, number>()),
    [prefs],
  );

  const scales = useMemo(() => {
    const map = new Map<string, number>();
    if (!prefs || !magOn || localX == null) return map;
    for (const item of prefs.items) {
      if (item.kind === "separator") continue;
      const c = centers.get(item.id);
      if (c == null) {
        map.set(item.id, 1);
        continue;
      }
      map.set(item.id, fanScale(Math.abs(localX - c), maxScale));
    }
    return map;
  }, [prefs, magOn, localX, maxScale, centers]);

  const onBarPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!magOn) return;
    const bar = barRef.current;
    if (!bar) return;
    const left = bar.getBoundingClientRect().left;
    const x = e.clientX - left;
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      setLocalX(x);
    });
  };

  const onBarPointerLeave = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    setLocalX(null);
  };

  async function onItemClick(item: DockItem) {
    if (item.kind === "separator") return;
    try {
      await invoke("dock_launch_item", { itemId: item.id });
      const win = getCurrentWindow();
      void win.setFocus().catch(() => undefined);
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
      <div className="dock-stack">
        <div className="dock-chrome" aria-hidden />
        <div
          ref={barRef}
          className="dock-bar"
          onPointerMove={onBarPointerMove}
          onPointerLeave={onBarPointerLeave}
        >
          {prefs.items.map((item) => {
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
            const style = {
              ["--dock-scale" as string]: String(scale),
              width: `${ICON_SLOT * scale}px`,
            } as CSSProperties;
            return (
              <button
                key={item.id}
                type="button"
                className={`dock-item${running ? " is-running" : ""}${scale > 1.02 ? " is-magnified" : ""}`}
                title={label}
                style={style}
                onClick={() => void onItemClick(item)}
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
