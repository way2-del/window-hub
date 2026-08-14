import {
  useCallback,
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
  /** Ephemeral running-app entry (not persisted). */
  ephemeral?: boolean;
  /** Live HWND for ephemeral focus (stable across title changes). */
  hwnd?: number | null;
};

type DockPrefs = {
  enabled: boolean;
  displayMode: string;
  hideSystemTaskbar: boolean;
  items: DockItem[];
  hotkey: string;
  magnification?: number;
  showPreview?: boolean;
  showHoverLabel?: boolean;
  cornerRadius?: number;
  bounceOnClick?: boolean;
  iconSize?: number;
  /** Gap between icon slots (4–24). */
  iconGap?: number;
  showTriggerStrip?: boolean;
  showRunningApps?: boolean;
  indicatorStyle?: string;
};

type HubWindow = {
  id: string;
  hwnd: number;
  title: string;
  /** Win32 class — snake_case from Rust WindowInfo. */
  class_name?: string | null;
  className?: string | null;
  exe?: string | null;
  /** Prefer snake_case (`exe_name`); camelCase kept for resilience. */
  exe_name?: string | null;
  exeName?: string | null;
};

const STATUS_MENU_W = 220;
const STATUS_MENU_H = 465;
const STATUS_MENU_GAP = 8;
const STATUS_MENU_MARGIN = 8;
const MAG_RANGE = 2.25;
const PREVIEW_DELAY_MS = 80;
const PREVIEW_CLOSE_MS = 220;

/** Never surface these as ephemeral running apps. */
const SKIP_EXE = new Set([
  "searchhost.exe",
  "searchui.exe",
  "startmenuexperiencehost.exe",
  "shellexperiencehost.exe",
  // applicationframehost kept — UWP apps need a dock slot (title used as label)
  "systemsettings.exe",
  "textinputhost.exe",
  "lockapp.exe",
  "dwm.exe",
  "sihost.exe",
  "runtimebroker.exe",
  "taskmgr.exe",
  // Suite / browser helpers — not real user windows (Rust also filters these).
  "wpscloudsvr.exe",
  "ksolaunch.exe",
  "ksomisc.exe",
  "wpscenter.exe",
  "wpsofficeboot.exe",
  "spotifylauncher.exe",
  "dingtalk_launcher.exe",
  "feishulauncher.exe",
  "updat.exe",
  "crashpad_handler.exe",
  "msedgewebview2.exe",
  "widgetservice.exe",
  "widgets.exe",
  "phoneexperiencehost.exe",
  "gamebar.exe",
  "gamebarftserver.exe",
  "xboxgamebar.exe",
]);

/** Explorer folder windows only — desktop/tray hosts must not light the pin. */
const EXPLORER_FOLDER_CLASSES = new Set(["cabinetwclass", "explorewclass"]);

/** This host process — hide from dock pins + running extras. */
const HOST_EXE = new Set([
  "window-hub.exe",
  "window_hub.exe",
  "windowshub.exe",
  "win-hub.exe",
  "winhub.exe",
]);

function isHostExeName(name: string): boolean {
  const n = name.trim().toLowerCase();
  if (!n) return false;
  const withExt = n.endsWith(".exe") ? n : `${n}.exe`;
  return HOST_EXE.has(withExt);
}

function windowExeName(w: HubWindow): string {
  return (w.exe_name || w.exeName || "").trim();
}

function windowClassName(w: HubWindow): string {
  return (w.class_name || w.className || "").trim();
}

function clampMagnification(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return 1.6;
  return Math.min(2.5, Math.max(1, n));
}

function clampRadius(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return 16;
  return Math.min(28, Math.max(8, Math.round(n)));
}

function clampIconSize(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return 40;
  return Math.min(56, Math.max(28, Math.round(n)));
}

function clampIconGap(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return 10;
  return Math.min(24, Math.max(4, Math.round(n)));
}

function fanScale(distancePx: number, maxScale: number, iconSlot: number): number {
  if (maxScale <= 1.001) return 1;
  const reach = iconSlot * MAG_RANGE;
  if (reach <= 0 || distancePx >= reach) return 1;
  const t = distancePx / reach;
  const w = Math.cos((t * Math.PI) / 2);
  return 1 + (maxScale - 1) * w * w;
}

function restingCenters(
  items: DockItem[],
  iconSlot: number,
  gap: number,
  padX: number,
  sepW: number,
) {
  const map = new Map<string, number>();
  let x = padX;
  items.forEach((item, i) => {
    if (i > 0) x += gap;
    if (item.kind === "separator") {
      x += sepW;
      return;
    }
    map.set(item.id, x + iconSlot / 2);
    x += iconSlot;
  });
  return map;
}

function contentWidth(
  items: DockItem[],
  iconSlot: number,
  gap: number,
  padX: number,
  sepW: number,
) {
  let w = padX * 2;
  items.forEach((item, i) => {
    if (i > 0) w += gap;
    w += item.kind === "separator" ? sepW : iconSlot;
  });
  return Math.max(100, w);
}

const SUITE_FAMILIES: string[][] = [
  [
    "wps.exe",
    "wpp.exe",
    "et.exe",
    "wpspdf.exe",
    "ksolaunch.exe",
    "wpsoffice.exe",
    "wpscloudsvr.exe",
    "ksomisc.exe",
    "wpscenter.exe",
    "wpsofficeboot.exe",
  ],
  ["wemeetapp.exe", "wemeetapp_new.exe", "tencentmeeting.exe"],
  ["dingtalk.exe", "dingtalk_launcher.exe"],
  ["feishu.exe", "lark.exe", "feishulauncher.exe"],
  ["spotify.exe", "spotifylauncher.exe"],
  ["discord.exe", "updat.exe"],
];

const DISTINCT_APPS: string[][] = [
  [
    "winword.exe",
    "excel.exe",
    "powerpnt.exe",
    "outlook.exe",
    "onenote.exe",
    "onenoteim.exe",
    "msaccess.exe",
    "mspub.exe",
    "teams.exe",
    "ms-teams.exe",
  ],
  [
    "idea64.exe",
    "idea.exe",
    "goland64.exe",
    "goland.exe",
    "webstorm64.exe",
    "webstorm.exe",
    "pycharm64.exe",
    "pycharm.exe",
    "clion64.exe",
    "phpstorm64.exe",
    "rider64.exe",
    "datagrip64.exe",
    "studio64.exe",
  ],
  ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe"],
];

const PRODUCT_MARKERS = [
  "\\kingsoft\\",
  "\\wps office\\",
  "\\dingding\\",
  "\\dingtalk\\",
  "\\feishu\\",
  "\\lark\\",
  "\\wemeet\\",
  "\\spotify\\",
  "\\discord\\",
];

const GENERIC_ROOTS = new Set([
  "program files",
  "program files (x86)",
  "windows",
  "system32",
  "syswow64",
  "users",
  "appdata",
  "local",
  "roaming",
  "locallow",
  "common files",
  "programdata",
  "programs",
]);

function normExeKey(raw: string): string {
  let s = (raw || "").trim().toLowerCase().replace(/\//g, "\\");
  if (!s) return "";
  const base = s.includes("\\") ? s.split("\\").pop() || s : s;
  if (!base) return "";
  if (base.endsWith(".exe")) return base;
  if (base.includes(".")) return base;
  return `${base}.exe`;
}

function pathParts(path: string): string[] {
  return (path || "")
    .toLowerCase()
    .replace(/\//g, "\\")
    .split("\\")
    .filter(Boolean);
}

function inSuiteTogether(pinExe: string, runExe: string): boolean {
  if (!pinExe || !runExe) return false;
  for (const family of SUITE_FAMILIES) {
    if (family.includes(pinExe) && family.includes(runExe)) return true;
  }
  return false;
}

function sameInstallTree(pinPath: string, runPath: string): boolean {
  const pinExe = normExeKey(pinPath);
  const runExe = normExeKey(runPath);
  if (pinExe && runExe && pinExe !== runExe) {
    for (const group of DISTINCT_APPS) {
      if (group.includes(pinExe) && group.includes(runExe)) return false;
    }
    // Different exe names only merge inside a known suite (WPS editor ↔ launcher).
    if (!inSuiteTogether(pinExe, runExe)) return false;
  }
  const a = pathParts(pinPath);
  const b = pathParts(runPath);
  if (a.length < 2 || b.length < 2) return false;
  const aDirs = a.slice(0, -1);
  const bDirs = b.slice(0, -1);
  let n = 0;
  while (n < aDirs.length && n < bDirs.length && aDirs[n] === bDirs[n]) n += 1;
  if (n === 0) return false;
  const meaningful = aDirs.slice(0, n).filter((c) => !GENERIC_ROOTS.has(c)).length;
  // Require 2 meaningful segments so Local\Programs alone cannot merge unrelated apps.
  return meaningful >= 2 && n >= 3;
}

function sameProductMarker(pinPath: string, runPath: string): boolean {
  const pinExe = normExeKey(pinPath);
  const runExe = normExeKey(runPath);
  if (pinExe && runExe && pinExe !== runExe && !inSuiteTogether(pinExe, runExe)) {
    return false;
  }
  const real = pinPath.toLowerCase().replace(/\//g, "\\");
  const path = runPath.toLowerCase().replace(/\//g, "\\");
  return PRODUCT_MARKERS.some((m) => real.includes(m) && path.includes(m));
}

function collectItemKeys(item: DockItem): Set<string> {
  const keys = new Set<string>();
  const add = (raw: string) => {
    const k = normExeKey(raw);
    if (k) keys.add(k);
  };
  add(item.matchExe);
  add(item.realPath);
  if (item.launchPath && !/\.lnk$/i.test(item.launchPath)) add(item.launchPath);

  const label = (item.label || "").toLowerCase();
  const blob = `${item.matchExe} ${item.realPath} ${item.launchPath} ${item.label}`.toLowerCase();
  for (const family of SUITE_FAMILIES) {
    let hit = false;
    for (const k of keys) {
      if (family.includes(k)) {
        hit = true;
        break;
      }
    }
    if (!hit) {
      hit = family.some((e) => {
        const stem = e.replace(/\.exe$/, "");
        return label.includes(stem) || blob.includes(stem);
      });
    }
    if (!hit && family[0] === "wps.exe") {
      hit = label.includes("wps") || label.includes("金山") || blob.includes("kingsoft");
    }
    if (hit) for (const e of family) keys.add(e);
  }
  return keys;
}

function itemSuites(item: DockItem, keys: Set<string>): string[][] {
  const out: string[][] = [];
  const label = (item.label || "").toLowerCase();
  const blob = `${item.matchExe} ${item.realPath} ${item.launchPath} ${item.label}`.toLowerCase();
  for (const family of SUITE_FAMILIES) {
    let hit = [...keys].some((k) => family.includes(k));
    if (!hit) {
      hit = family.some((e) => {
        const stem = e.replace(/\.exe$/, "");
        return label.includes(stem) || blob.includes(stem);
      });
    }
    if (!hit && family[0] === "wps.exe") {
      hit = label.includes("wps") || label.includes("金山") || blob.includes("kingsoft");
    }
    if (hit) out.push(family);
  }
  return out;
}

function exeMatches(item: DockItem, w: HubWindow): boolean {
  const name = normExeKey(windowExeName(w));
  const path = (w.exe || "").toLowerCase().replace(/\//g, "\\");
  const pathBase = normExeKey(path.split("\\").pop() || "");
  const className = windowClassName(w).toLowerCase();
  const keys = collectItemKeys(item);
  const suites = itemSuites(item, keys);

  // explorer.exe: only real folder windows light the pin / count as running.
  if (name === "explorer.exe" || pathBase === "explorer.exe") {
    if (!EXPLORER_FOLDER_CLASSES.has(className)) return false;
    const pinLooksExplorer =
      keys.has("explorer.exe") ||
      (item.label || "").toLowerCase().includes("explorer") ||
      (item.label || "").includes("资源管理器") ||
      (item.matchExe || "").toLowerCase().includes("explorer");
    return pinLooksExplorer;
  }

  if (name && keys.has(name)) return true;
  if (pathBase && keys.has(pathBase)) return true;

  const real = (item.realPath || "").toLowerCase().replace(/\//g, "\\");
  if (real && path && real === path) return true;
  if (real && path) {
    const rd = real.slice(0, real.lastIndexOf("\\"));
    const pd = path.slice(0, path.lastIndexOf("\\"));
    if (
      rd &&
      rd === pd &&
      !rd.includes("\\windows\\") &&
      !rd.includes("\\system32") &&
      !rd.includes("\\syswow64")
    ) {
      const pinExe = normExeKey(real);
      // Same folder: same exe, or suite pair (launcher + editor).
      if (!pathBase || pinExe === pathBase || inSuiteTogether(pinExe, pathBase)) {
        return true;
      }
    }
    if (sameProductMarker(real, path)) return true;
    if (sameInstallTree(real, path)) return true;
  }
  // Also try launch path as tree root when real_path empty.
  const launch = (item.launchPath || "").toLowerCase().replace(/\//g, "\\");
  if (launch.endsWith(".exe") && path && sameInstallTree(launch, path)) return true;

  for (const family of suites) {
    if (name && family.includes(name)) return true;
    if (pathBase && family.includes(pathBase)) return true;
  }
  // Soft title match only for WPS document titles — generic label⊂title
  // wrongly absorbs unrelated windows into pins (hides unpinned extras).
  if (suites.some((f) => f[0] === "wps.exe")) {
    const title = (w.title || "").toLowerCase();
    if (title.includes("wps") || title.includes("金山")) return true;
  }
  return false;
}

/**
 * Strict pin occupancy check for ephemeral running-apps.
 * Tree / marker / soft-title matching must NOT hide unpinned apps from the dock.
 */
function pinOwnsWindow(item: DockItem, w: HubWindow): boolean {
  const name = normExeKey(windowExeName(w));
  const path = (w.exe || "").toLowerCase().replace(/\//g, "\\");
  const pathBase = normExeKey(path.split("\\").pop() || "");
  const className = windowClassName(w).toLowerCase();
  const keys = collectItemKeys(item);

  if (name === "explorer.exe" || pathBase === "explorer.exe") {
    if (!EXPLORER_FOLDER_CLASSES.has(className)) return false;
    return (
      keys.has("explorer.exe") ||
      (item.label || "").toLowerCase().includes("explorer") ||
      (item.label || "").includes("资源管理器") ||
      (item.matchExe || "").toLowerCase().includes("explorer")
    );
  }

  if (name && keys.has(name)) return true;
  if (pathBase && keys.has(pathBase)) return true;
  const real = (item.realPath || "").toLowerCase().replace(/\//g, "\\");
  if (real && path && real === path) return true;

  const suites = itemSuites(item, keys);
  for (const family of suites) {
    if (name && family.includes(name)) return true;
    if (pathBase && family.includes(pathBase)) return true;
  }
  return false;
}

function itemLabel(item: DockItem): string {
  if (item.kind === "startmenu") return "开始";
  if (item.kind === "trash") return "回收站";
  return item.label || item.matchExe || item.id;
}

function normalizeExeName(w: HubWindow): string {
  const name = windowExeName(w).toLowerCase();
  if (name) return name.endsWith(".exe") ? name : `${name}.exe`;
  const path = (w.exe || "").replace(/\\/g, "/");
  const base = path.split("/").pop() || "";
  return base.toLowerCase();
}

/** UWP host: keep one ephemeral slot keyed by title stem, not by ApplicationFrameHost. */
function ephemeralKey(w: HubWindow): string {
  const exeName = normalizeExeName(w);
  if (exeName === "applicationframehost.exe") {
    const stem =
      (w.title || "").split(/[-—|·]/)[0]?.trim().toLowerCase() || "uwp";
    return `uwp:${stem}`;
  }
  return (w.exe || exeName).toLowerCase();
}

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

async function clientToScreen(clientX: number, clientY: number) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  return {
    x: outer.x / factor + clientX,
    y: outer.y / factor + clientY,
  };
}

export default function DockApp() {
  const [prefs, setPrefs] = useState<DockPrefs | null>(null);
  const [windows, setWindows] = useState<HubWindow[]>([]);
  const [localX, setLocalX] = useState<number | null>(null);
  const [bounceId, setBounceId] = useState<string | null>(null);
  const [exeIcons, setExeIcons] = useState<Record<string, string>>({});
  const shellRef = useRef<HTMLDivElement | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef(0);
  const previewOpenTimer = useRef(0);
  const previewCloseTimer = useRef(0);
  const bounceTimer = useRef(0);
  const previewItemId = useRef<string | null>(null);
  const previewPointerInside = useRef(false);
  const previewSuppressUntil = useRef(0);
  const lastExtraW = useRef(-1);
  const restingWidthRef = useRef(100);

  useEffect(() => {
    let cancelled = false;
    const applyMaterial = async () => {
      try {
        const material = await invoke<GlassPrefs>("get_material_prefs");
        const sysDark = await invoke<boolean>("system_apps_dark").catch(() => undefined);
        const compat = await invoke<boolean>("is_glass_compat_mode").catch(() => false);
        if (compat) {
          document.documentElement.dataset.glassCompat = "1";
          (window as Window & { __WH_GLASS_COMPAT__?: boolean }).__WH_GLASS_COMPAT__ = true;
        }
        applyGlassCss(
          {
            kind: "mica-alt",
            dark: material.dark,
            acrylicAlpha: material.acrylicAlpha,
          },
          sysDark,
          false,
        );
        if (compat) {
          document.documentElement.dataset.glassCompat = "1";
        }
        await invoke("apply_window_effect", {}).catch(() => undefined);
      } catch {
        /* noop */
      }
    };
    void applyMaterial();
    const retryA = window.setTimeout(() => void applyMaterial(), 150);
    const retryB = window.setTimeout(() => void applyMaterial(), 400);

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
    // Backup poll — event may coalesce/miss while Dock HWND is placing.
    const pollWindows = window.setInterval(() => {
      void invoke<HubWindow[]>("list_open_windows")
        .then((list) => {
          if (!cancelled) setWindows(list);
        })
        .catch(() => undefined);
    }, 1200);
    void listen("material-prefs", () => {
      void applyMaterial();
    }).then((u) => unsubs.push(u));
    void listen<{ inside?: boolean }>("dock-preview-pointer", (e) => {
      previewPointerInside.current = !!e.payload?.inside;
      if (e.payload?.inside) {
        if (previewCloseTimer.current) {
          window.clearTimeout(previewCloseTimer.current);
          previewCloseTimer.current = 0;
        }
      } else {
        if (previewCloseTimer.current) window.clearTimeout(previewCloseTimer.current);
        previewCloseTimer.current = window.setTimeout(() => {
          previewItemId.current = null;
          void invoke("close_dock_preview").catch(() => undefined);
        }, PREVIEW_CLOSE_MS);
      }
    }).then((u) => unsubs.push(u));

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      window.clearInterval(pollWindows);
      for (const u of unsubs) u();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      if (previewOpenTimer.current) window.clearTimeout(previewOpenTimer.current);
      if (previewCloseTimer.current) window.clearTimeout(previewCloseTimer.current);
      if (bounceTimer.current) window.clearTimeout(bounceTimer.current);
      void invoke("close_dock_preview").catch(() => undefined);
      void invoke("dock_set_runtime_extra_width", { extra: 0 }).catch(() => undefined);
    };
  }, []);

  const maxScale = clampMagnification(prefs?.magnification);
  const magOn = maxScale > 1.001;
  const showPreview = prefs?.showPreview !== false;
  const bounceOnClick = prefs?.bounceOnClick !== false;
  const showRunningApps =
    (prefs as DockPrefs & { show_running_apps?: boolean })?.showRunningApps !== false &&
    (prefs as DockPrefs & { show_running_apps?: boolean })?.show_running_apps !== false;
  const indicatorStyle = prefs?.indicatorStyle === "dot" ? "dot" : "bar";
  const cornerRadius = clampRadius(prefs?.cornerRadius);
  const iconSlot = clampIconSize(prefs?.iconSize);
  const iconPx = Math.round(iconSlot * 0.9);
  const gap = clampIconGap(prefs?.iconGap);
  const padX = 22;
  const sepW = 8;

  const activeIds = useMemo(() => {
    const set = new Set<string>();
    if (!prefs) return set;
    for (const item of prefs.items) {
      if (item.kind !== "app") continue;
      if (windows.some((w) => exeMatches(item, w))) set.add(item.id);
    }
    return set;
  }, [prefs, windows]);

  /** First-seen order for ephemeral icons — do not reshuffle when EnumWindows z-order changes. */
  const ephemeralOrderRef = useRef<string[]>([]);

  const runningExtras = useMemo(() => {
    if (!prefs || !showRunningApps) return [] as DockItem[];
    const pinned = prefs.items.filter((i) => i.kind === "app");
    const byKey = new Map<string, DockItem>();
    for (const w of windows) {
      const exeName = normalizeExeName(w);
      if (!exeName || SKIP_EXE.has(exeName) || isHostExeName(exeName)) continue;
      if (isHostExeName(windowExeName(w)) || isHostExeName(w.exe?.split(/[/\\]/).pop() || "")) {
        continue;
      }
      if (pinned.some((item) => pinOwnsWindow(item, w))) continue;
      const key = ephemeralKey(w);
      if (byKey.has(key)) continue;
      const isUwp = exeName === "applicationframehost.exe";
      const label =
        (w.title || "").split(/[-—|·]/)[0]?.trim() ||
        (windowExeName(w) || exeName.replace(/\.exe$/i, ""));
      byKey.set(key, {
        id: `running:${key}`,
        kind: "app",
        label,
        matchExe: isUwp ? "" : exeName,
        launchPath: w.exe || "",
        realPath: w.exe || "",
        virtualPath: "",
        iconPath: "",
        uwp: isUwp,
        iconPng: exeIcons[key] || exeIcons[exeName] || null,
        ephemeral: true,
        hwnd: w.hwnd,
      });
    }
    const alive = new Set(byKey.keys());
    const order = ephemeralOrderRef.current.filter((k) => alive.has(k));
    for (const k of byKey.keys()) {
      if (!order.includes(k)) order.push(k);
    }
    ephemeralOrderRef.current = order;
    return order.map((k) => byKey.get(k)!);
  }, [prefs, windows, showRunningApps, exeIcons]);

  // Resolve icons for ephemeral running apps — deferred so first paint stays responsive.
  useEffect(() => {
    let cancelled = false;
    const missing = runningExtras.filter((it) => {
      const key = (it.realPath || it.matchExe).toLowerCase();
      return !it.iconPng && (it.realPath || it.matchExe) && !exeIcons[key];
    });
    if (!missing.length) return;
    const timer = window.setTimeout(() => {
      void (async () => {
        const next: Record<string, string> = {};
        // One-at-a-time, capped — Shell extract must never pile onto startup.
        for (const it of missing.slice(0, 6)) {
          if (cancelled) break;
          const path = it.realPath || it.matchExe;
          try {
            const png = await invoke<string | null>("dock_resolve_exe_icon", { path });
            if (png) {
              next[(it.realPath || it.matchExe).toLowerCase()] = png;
              next[it.matchExe.toLowerCase()] = png;
            }
          } catch {
            /* noop */
          }
        }
        if (!cancelled && Object.keys(next).length) {
          setExeIcons((prev) => ({ ...prev, ...next }));
        }
      })();
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [runningExtras, exeIcons]);

  const displayItems = useMemo(() => {
    if (!prefs) return [] as DockItem[];
    // Never show the host app as a dock icon (pinned or ephemeral).
    const pinned = prefs.items.filter(
      (i) =>
        i.kind !== "app" ||
        (!isHostExeName(i.matchExe) &&
          !isHostExeName(i.realPath.split(/[/\\]/).pop() || "") &&
          !isHostExeName(i.launchPath.split(/[/\\]/).pop() || "")),
    );
    if (!runningExtras.length) return pinned;
    const needsSep =
      pinned.length > 0 &&
      pinned[pinned.length - 1]?.kind !== "separator" &&
      !pinned.some((i) => i.id === "__runtime_sep__");
    const sep: DockItem = {
      id: "__runtime_sep__",
      kind: "separator",
      label: "",
      matchExe: "",
      launchPath: "",
      realPath: "",
      virtualPath: "",
      iconPath: "",
      uwp: false,
      ephemeral: true,
    };
    return needsSep ? [...pinned, sep, ...runningExtras] : [...pinned, ...runningExtras];
  }, [prefs, runningExtras]);

  const pinnedWidth = useMemo(
    () => (prefs ? contentWidth(prefs.items, iconSlot, gap, padX, sepW) : 100),
    [prefs, iconSlot, gap],
  );
  const fullWidth = useMemo(
    () => contentWidth(displayItems, iconSlot, gap, padX, sepW),
    [displayItems, iconSlot, gap],
  );
  restingWidthRef.current = fullWidth;

  useEffect(() => {
    const extra = Math.max(0, fullWidth - pinnedWidth);
    if (Math.abs(extra - lastExtraW.current) < 0.5) return;
    lastExtraW.current = extra;
    const t = window.setTimeout(() => {
      void invoke("dock_set_runtime_extra_width", { extra }).catch(() => undefined);
    }, 80);
    return () => window.clearTimeout(t);
  }, [fullWidth, pinnedWidth]);

  const centers = useMemo(
    () => restingCenters(displayItems, iconSlot, gap, padX, sepW),
    [displayItems, iconSlot, gap],
  );

  const scales = useMemo(() => {
    const map = new Map<string, number>();
    if (!magOn || localX == null) return map;
    for (const item of displayItems) {
      if (item.kind === "separator") continue;
      const c = centers.get(item.id);
      if (c == null) {
        map.set(item.id, 1);
        continue;
      }
      map.set(item.id, fanScale(Math.abs(localX - c), maxScale, iconSlot));
    }
    return map;
  }, [displayItems, magOn, localX, maxScale, centers, iconSlot]);

  // Pill = icon + pads (bottom reserves room for green mark).
  // Transparent headroom above so magnification can rise out of the pill.
  const chromeBase = Math.round(iconSlot + 18);
  const chromeH = chromeBase;
  const magHead = magOn ? Math.ceil(iconSlot * (maxScale - 1) * 1.1) : 0;
  const stackH = chromeBase + magHead;

  useEffect(() => {
    void invoke("dock_set_extra_headroom", { px: magHead }).catch(() => undefined);
  }, [magHead]);

  const closePreviewSoon = useCallback(() => {
    if (previewOpenTimer.current) {
      window.clearTimeout(previewOpenTimer.current);
      previewOpenTimer.current = 0;
    }
    if (previewCloseTimer.current) window.clearTimeout(previewCloseTimer.current);
    previewCloseTimer.current = window.setTimeout(() => {
      if (previewPointerInside.current) return;
      previewItemId.current = null;
      void invoke("close_dock_preview").catch(() => undefined);
    }, PREVIEW_CLOSE_MS);
  }, []);

  const openPreviewFor = useCallback(
    (item: DockItem, el: HTMLElement) => {
      const running = item.ephemeral || activeIds.has(item.id);
      if (!showPreview || item.kind !== "app" || !running) {
        return;
      }
      if (Date.now() < previewSuppressUntil.current) {
        return;
      }
      if (previewCloseTimer.current) {
        window.clearTimeout(previewCloseTimer.current);
        previewCloseTimer.current = 0;
      }
      if (previewOpenTimer.current) window.clearTimeout(previewOpenTimer.current);
      previewOpenTimer.current = window.setTimeout(() => {
        void (async () => {
          try {
            if (Date.now() < previewSuppressUntil.current) {
              return;
            }
            if (previewItemId.current === item.id) {
              return;
            }
            const bar = barRef.current;
            const rect = el.getBoundingClientRect();
            const anchorY = bar ? bar.getBoundingClientRect().top : rect.top;
            const mid = await clientToScreen(rect.left + rect.width / 2, anchorY);
            if (Date.now() < previewSuppressUntil.current) {
              return;
            }
            previewItemId.current = item.id;
            await invoke("open_dock_preview", {
              itemId: item.id,
              anchorX: mid.x,
              anchorY: mid.y,
              matchExe: item.matchExe || null,
              realPath: item.realPath || null,
              label: itemLabel(item),
              iconPng: item.iconPng || null,
            });
          } catch (e) {
            console.error(e);
          }
        })();
      }, PREVIEW_DELAY_MS);
    },
    [showPreview, activeIds],
  );

  const onBarPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!magOn) return;
    const shell = shellRef.current;
    if (!shell) return;
    // Use RESTING content left (shell-centered), never live bar width —
    // otherwise magnifying shifts bar.left and the fan oscillates.
    const shellRect = shell.getBoundingClientRect();
    const restingW = restingWidthRef.current;
    const contentLeft = shellRect.left + (shellRect.width - restingW) / 2;
    const x = e.clientX - contentLeft;
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      setLocalX(x);
    });
  };

  const onBarPointerLeave = () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    setLocalX(null);
    closePreviewSoon();
  };

  async function onItemClick(item: DockItem) {
    if (item.kind === "separator") return;
    if (previewOpenTimer.current) {
      window.clearTimeout(previewOpenTimer.current);
      previewOpenTimer.current = 0;
    }
    if (previewCloseTimer.current) {
      window.clearTimeout(previewCloseTimer.current);
      previewCloseTimer.current = 0;
    }
    previewItemId.current = null;
    previewPointerInside.current = false;
    previewSuppressUntil.current = Date.now() + 700;
    // Fire-and-forget teardown — never block launch on preview/menu IPC.
    void invoke("close_dock_preview").catch(() => undefined);
    void invoke("close_dock_item_menu").catch(() => undefined);
    if (bounceOnClick) {
      setBounceId(item.id);
      if (bounceTimer.current) window.clearTimeout(bounceTimer.current);
      bounceTimer.current = window.setTimeout(() => setBounceId(null), 560);
    }
    try {
      if (item.ephemeral) {
        const want = (item.matchExe || "").toLowerCase();
        const key = item.id.replace(/^running:/, "");
        // Prefer stored HWND — EnumWindows order / title changes must not drop the click.
        const matched =
          (item.hwnd != null
            ? windows.find((w) => w.hwnd === item.hwnd) || {
                id: `hwnd:${item.hwnd}`,
                hwnd: item.hwnd,
                title: item.label,
              }
            : null) ||
          windows.find((w) => ephemeralKey(w) === key) ||
          windows.find((w) => exeMatches(item, w)) ||
          windows.find((w) => {
            const n = normalizeExeName(w);
            return !!want && (n === want || n === `${want.replace(/\.exe$/i, "")}.exe`);
          });
        // Focus / minimize toggle — never ShellExecute while a window exists
        // (Cursor / Electron would open a brand-new window).
        if (matched?.id) {
          void invoke("focus_or_minimize_open_window", { id: matched.id }).catch((e) =>
            console.error(e),
          );
        } else if (item.realPath) {
          void invoke("dock_launch_path", { path: item.realPath }).catch((e) => console.error(e));
        } else {
          console.error("ephemeral click: no hwnd and no path", item);
        }
      } else {
        void invoke("dock_launch_item", { itemId: item.id }).catch((e) => console.error(e));
      }
    } catch (e) {
      console.error(e);
    }
  }

  async function openItemContextMenu(e: MouseEvent, item: DockItem) {
    e.preventDefault();
    e.stopPropagation();
    previewItemId.current = null;
    void invoke("close_dock_preview").catch(() => undefined);
    if (item.ephemeral) return;
    try {
      const screen = await clientToScreen(e.clientX, e.clientY);
      const menuH = 260;
      let x = screen.x;
      let y = screen.y - menuH - 8;
      if (y < 8) y = screen.y + 8;
      await invoke("open_dock_item_menu", { itemId: item.id, x, y });
    } catch (err) {
      console.error(err);
    }
  }

  function onBackgroundContextMenu(e: MouseEvent) {
    e.preventDefault();
    const target = e.target as HTMLElement | null;
    if (target?.closest(".dock-item") || target?.closest(".dock-sep")) {
      return;
    }
    previewItemId.current = null;
    void invoke("close_dock_preview").catch(() => undefined);
    void invoke("close_dock_item_menu").catch(() => undefined);
    void openStatusMenuAtClientPoint(e.clientX, e.clientY).catch((err) => {
      console.error(err);
    });
  }

  if (!prefs) {
    return <div className="dock-shell dock-loading" />;
  }

  const shellStyle = {
    ["--dock-radius" as string]: `${cornerRadius}px`,
    ["--dock-h" as string]: `${stackH}px`,
    ["--dock-chrome-h" as string]: `${chromeH}px`,
    ["--dock-icon-slot" as string]: `${iconSlot}px`,
    ["--dock-icon" as string]: `${iconPx}px`,
    ["--dock-mag-max" as string]: String(maxScale),
    ["--dock-gap" as string]: `${gap}px`,
    ["--dock-pad-x" as string]: `${padX}px`,
  } as CSSProperties;

  return (
    <div
      ref={shellRef}
      className="dock-shell"
      data-mode={prefs.displayMode}
      data-mag={magOn ? "on" : "off"}
      data-indicator={indicatorStyle}
      style={shellStyle}
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
          {displayItems.map((item) => {
            if (item.kind === "separator") {
              return <span key={item.id} className="dock-sep" aria-hidden />;
            }
            const running = item.ephemeral || activeIds.has(item.id);
            const scale = scales.get(item.id) ?? 1;
            const label = itemLabel(item);
            // Mild neighbor spacing only — large margins push edge icons past the pill.
            // Overflow is clipped by .dock-stack; keep grow small so clips rarely happen.
            const grow = Math.max(0, iconSlot * (scale - 1) * 0.35);
            const style = {
              ["--dock-scale" as string]: String(scale),
              width: `${iconSlot}px`,
              marginLeft: `${grow / 2}px`,
              marginRight: `${grow / 2}px`,
            } as CSSProperties;
            return (
              <div
                key={item.id}
                role="button"
                tabIndex={-1}
                className={`dock-item${running ? " is-running" : ""}${
                  scale > 1.02 ? " is-magnified" : ""
                }${bounceId === item.id ? " is-bounce" : ""}`}
                style={style}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  // Fire on press — pointerup misses when mag scale moves the target under the cursor.
                  e.stopPropagation();
                  try {
                    (document.activeElement as HTMLElement | null)?.blur();
                  } catch {
                    /* noop */
                  }
                  void onItemClick(item);
                }}
                onContextMenu={(e) => void openItemContextMenu(e, item)}
                onPointerEnter={(e) => {
                  openPreviewFor(item, e.currentTarget);
                }}
                onPointerLeave={() => {
                  closePreviewSoon();
                }}
              >
                <span className="dock-col">
                  <span className="dock-hit">
                    <span className="dock-glyph">
                      {item.iconPng ? (
                        <img
                          className="dock-icon"
                          src={`data:image/png;base64,${item.iconPng}`}
                          alt=""
                          draggable={false}
                        />
                      ) : item.kind === "startmenu" ? (
                        <span className="dock-icon-glyph dock-icon-start" aria-hidden>
                          <svg viewBox="0 0 24 24" width="22" height="22">
                            <rect x="2" y="2" width="9" height="9" rx="1.2" fill="currentColor" />
                            <rect x="13" y="2" width="9" height="9" rx="1.2" fill="currentColor" />
                            <rect x="2" y="13" width="9" height="9" rx="1.2" fill="currentColor" />
                            <rect x="13" y="13" width="9" height="9" rx="1.2" fill="currentColor" />
                          </svg>
                        </span>
                      ) : item.kind === "trash" ? (
                        <span className="dock-icon-glyph dock-icon-trash" aria-hidden>
                          <svg viewBox="0 0 24 24" width="22" height="22" fill="none">
                            <path
                              d="M8 7h8l-.7 12.2a1.5 1.5 0 0 1-1.5 1.4h-3.6a1.5 1.5 0 0 1-1.5-1.4L8 7Z"
                              stroke="currentColor"
                              strokeWidth="1.6"
                            />
                            <path
                              d="M6.5 7h11M10 7V5.8A1.8 1.8 0 0 1 11.8 4h.4A1.8 1.8 0 0 1 14 5.8V7"
                              stroke="currentColor"
                              strokeWidth="1.6"
                              strokeLinecap="round"
                            />
                          </svg>
                        </span>
                      ) : (
                        <span className="dock-icon-fallback" aria-hidden>
                          {(label || "?").charAt(0)}
                        </span>
                      )}
                    </span>
                  </span>
                  <span className="dock-foot" aria-hidden>
                    <span
                      className={`dock-indicator${running ? " is-on" : ""}${
                        indicatorStyle === "dot" ? " is-dot" : " is-bar"
                      }`}
                    />
                  </span>
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
