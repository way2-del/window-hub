import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type KeyboardEvent as ReactKeyboardEvent, type DragEvent as ReactDragEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import {
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
} from "@tauri-apps/api/window";
import { BorderBeam } from "border-beam";
import TrayCluster from "./components/TrayCluster";
import ShortcutsHost from "./components/ShortcutsHost";
import StatusMenu from "./components/StatusMenu";
import IslandPanelHost from "./components/IslandPanelHost";
import {
  applyIslandPrefsSnapshot,
  getIslandPrefs,
  hydrateIslandPrefs,
  subscribeIslandPrefs,
  clampStagingPanelH,
  clampStagingPanelW,
  STAGING_PANEL_H_DEFAULT,
  type IslandPrefs,
  mergeBarPriority,
} from "./islandPrefs";
import { islandNotifyBus, type IslandNotifyBanner } from "./plugins/islandNotify";
import {
  actionsForSlot,
  normalizeNotifyActions,
  type NotifyAction,
} from "./plugins/notifyActions";
import { bootstrapPlugins, subscribeInstalledPlugins } from "./plugins/bootstrap";
import { normalizeStagingChanged } from "./stagingApi";
import {
  formatStagingBarText,
  isStagingPanelShell,
  listBarResidentProviders,
  pickIslandContentBar,
  resolveIslandDropPluginId,
  resolvePluginPanelShellSize,
  type IslandBarState,
} from "./plugins/islandSlots";
import { parsePluginPanelId } from "./plugins/panelProviders";
import { pluginRegistry } from "./plugins/registry";
import "./App.css";

/** 默认插件面板展开尺寸（非中转站） */
const VIEW_W_DEFAULT = 380;
const VIEW_H_DEFAULT = 220;
/** 岛贴屏顶后顶隙为 0；窗口高度 = 岛高 */
const TOP_GAP = 0;
const ISLAND_COLLAPSED = { width: 300, height: 28 };
/** 当前展开目标 / SVG 画布（中转站时变宽变矮）——由 App 每帧同步 */
const liveExpanded = { width: VIEW_W_DEFAULT, height: VIEW_H_DEFAULT };
const HEIGHT_MS = 280;
/** 展开/收起总时长：宽高交错，禁止出现「380×28 宽扁直角条」中间态 */
const MORPH_MS = 420;
const PULL_OPEN = 0.52;
const CLICK_SLOP = 6;
const SPRING_MS = 320;

type IslandSize = { width: number; height: number };

/** 窗口实际高度 = 岛高（贴顶，无额外顶隙） */
function winHeight(islandH: number) {
  return TOP_GAP + islandH;
}

function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t;
}

function clamp01(t: number) {
  return Math.max(0, Math.min(1, t));
}

/** 下拉进度：原位点不动，弹窗高度随拖拽增大 */
function pullProgress(dy: number) {
  if (dy <= 0) return 0;
  const t = clamp01(dy / 210);
  // 阻力：越拉越沉
  return 1 - Math.pow(1 - t, 1.85);
}

function sizeFromProgress(p: number): IslandSize {
  const t = clamp01(p);
  return {
    // 跟手用亚像素，避免取整造成顶部黑条一顿一顿
    width: lerp(ISLAND_COLLAPSED.width, liveExpanded.width, t),
    height: lerp(ISLAND_COLLAPSED.height, liveExpanded.height, t),
  };
}

/** 岛底圆角半径（与 islandPath 共用，供 BorderBeam 贴合） */
function islandBottomRadius(width: number, height: number): number {
  const w = Math.max(28, width);
  const h = Math.max(28, height);
  const raw = Math.min(
    h * 0.5 - 0.01,
    Math.max(14, 14 + ((h - 28) * 18) / 192),
    w * 0.5 - 4,
  );
  // 较矮面板：底角过大时会切掉四角内容
  if (h <= STAGING_PANEL_H_DEFAULT + 4) return Math.min(raw, 18);
  return Math.min(raw, 32);
}

/**
 * 灵动岛路径（本地坐标：左上为 0,0，宽高=当前岛尺寸）。
 * 禁止再嵌进更大的「画布居中」坐标系，否则折叠宽与展开画布不一致时黑壳会偏/歪。
 * topSquare≥1：顶角真直角贴边；底角始终圆角。
 */
function islandPath(width: number, height: number, topSquare = 0): string {
  const w = Math.max(28, width);
  const h = Math.max(28, height);
  const x0 = 0;
  const x1 = w;

  const rBot = islandBottomRadius(w, h);
  const flat = clamp01(topSquare);
  const squareTop = flat >= 0.999;
  const rTop = squareTop ? 0 : Math.max(0.05, rBot * (1 - flat));
  const k = 0.5522847498;
  const rkBot = rBot * k;
  const y0 = 0;
  const y1 = h;
  const sideBot = y1 - rBot;

  if (squareTop) {
    // 顶边直角：纯直线拐角，不用贝塞尔
    return [
      `M ${fmt(x0)} ${fmt(y0)}`,
      `L ${fmt(x1)} ${fmt(y0)}`,
      `L ${fmt(x1)} ${fmt(sideBot)}`,
      `C ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1 - rBot)} ${fmt(y1)}`,
      `L ${fmt(x0 + rBot)} ${fmt(y1)}`,
      `C ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0)} ${fmt(sideBot)}`,
      `L ${fmt(x0)} ${fmt(y0)}`,
      `Z`,
    ].join(" ");
  }

  const rkTop = rTop * k;
  const sideTop = y0 + rTop;
  return [
    `M ${fmt(x0 + rTop)} ${fmt(y0)}`,
    `L ${fmt(x1 - rTop)} ${fmt(y0)}`,
    `C ${fmt(x1 - rTop + rkTop)} ${fmt(y0)}, ${fmt(x1)} ${fmt(y0 + rTop - rkTop)}, ${fmt(x1)} ${fmt(sideTop)}`,
    `L ${fmt(x1)} ${fmt(sideBot)}`,
    `C ${fmt(x1)} ${fmt(sideBot + rkBot)}, ${fmt(x1 - rBot + rkBot)} ${fmt(y1)}, ${fmt(x1 - rBot)} ${fmt(y1)}`,
    `L ${fmt(x0 + rBot)} ${fmt(y1)}`,
    `C ${fmt(x0 + rBot - rkBot)} ${fmt(y1)}, ${fmt(x0)} ${fmt(sideBot + rkBot)}, ${fmt(x0)} ${fmt(sideBot)}`,
    `L ${fmt(x0)} ${fmt(sideTop)}`,
    `C ${fmt(x0)} ${fmt(y0 + rTop - rkTop)}, ${fmt(x0 + rTop - rkTop)} ${fmt(y0)}, ${fmt(x0 + rTop)} ${fmt(y0)}`,
    `Z`,
  ].join(" ");
}

function fmt(n: number) {
  return (Math.round(n * 10) / 10).toString();
}

/** 近似 cubic-bezier(0.22, 1, 0.36, 1)：快起、尾段丝滑 */
function easeOutSmooth(t: number) {
  const x = clamp01(t);
  // 用 1-(1-x)^3 与 softer 混合，避免「砸到位」的顿挫
  const a = 1 - Math.pow(1 - x, 3);
  const b = x * x * (3 - 2 * x); // smoothstep
  return a * 0.72 + b * 0.28;
}

/** 把全局进度映射到 [start,end] 子区间，再 ease */
function channelEase(p: number, start: number, end: number) {
  return easeOutSmooth(clamp01((p - start) / Math.max(0.001, end - start)));
}

type Rgb = { r: number; g: number; b: number };

/** sRGB 相对亮度，用于顶栏文字黑白切换 */
function srgbLuma({ r, g, b }: Rgb) {
  const toLin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * toLin(r) + 0.7152 * toLin(g) + 0.0722 * toLin(b);
}

type ChromeTokens = {
  fg: string;
  fgHover: string;
  shadow: string;
  glyphShadow: string;
  scheme: "light" | "dark";
};

/** 浅色背景用纯黑字，深色背景用白字，避免顶栏看不见 */
function chromeTokens(rgb: Rgb): ChromeTokens {
  if (srgbLuma(rgb) >= 0.52) {
    return {
      fg: "#000000",
      fgHover: "#000000",
      shadow: "none",
      glyphShadow: "drop-shadow(0 0.5px 0.5px rgba(255, 255, 255, 0.7))",
      scheme: "light",
    };
  }
  return {
    fg: "rgba(255, 255, 255, 0.94)",
    fgHover: "#ffffff",
    shadow: "0 1px 2px rgba(0, 0, 0, 0.35)",
    glyphShadow: "drop-shadow(0 1px 1px rgba(0, 0, 0, 0.28))",
    scheme: "dark",
  };
}

function chromeCssVars(prefix: "left" | "right" | "center", t: ChromeTokens): Record<string, string> {
  return {
    [`--chrome-${prefix}-fg`]: t.fg,
    [`--chrome-${prefix}-fg-hover`]: t.fgHover,
    [`--chrome-${prefix}-shadow`]: t.shadow,
    [`--chrome-${prefix}-glyph-shadow`]: t.glyphShadow,
  };
}

/** 从色带 PNG 左 / 中 / 右采样，左右与岛中文字对比度各用一端 */
async function sampleStripBands(
  b64: string,
): Promise<{ left: Rgb; center: Rgb; right: Rgb } | null> {
  try {
    const img = new Image();
    img.decoding = "async";
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("png"));
      img.src = `data:image/png;base64,${b64}`;
    });
    const w = Math.max(1, img.naturalWidth);
    const h = Math.max(1, img.naturalHeight);
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0);
    const band = Math.max(1, Math.floor(w * 0.08));
    const avg = (x0: number, x1: number): Rgb => {
      const data = ctx.getImageData(x0, 0, Math.max(1, x1 - x0), h).data;
      let r = 0;
      let g = 0;
      let b = 0;
      let n = 0;
      for (let i = 0; i < data.length; i += 4) {
        r += data[i]!;
        g += data[i + 1]!;
        b += data[i + 2]!;
        n += 1;
      }
      return {
        r: Math.round(r / n),
        g: Math.round(g / n),
        b: Math.round(b / n),
      };
    };
    const mid0 = Math.max(0, Math.floor(w / 2 - band / 2));
    return {
      left: avg(0, band),
      center: avg(mid0, mid0 + band),
      right: avg(Math.max(0, w - band), w),
    };
  } catch {
    return null;
  }
}

type Material = "none";

type Ambient = {
  r: number;
  g: number;
  b: number;
  width?: number;
  offset_x?: number;
  span_width?: number;
  png_base64?: string;
};

type TrayAttention = {
  id: string;
  tooltip: string;
  process: string;
  icon_png_base64: string;
  hwnd: number;
  uid: number;
  callback_msg: number;
  version: number;
};

type MsgBanner = {
  key: string;
  text: string;
  iconPng: string;
  title: string;
  source: "tray" | "plugin";
  pluginId?: string;
  /** Tray icon id for invoke / clear flashing */
  trayIconId?: string;
  hwnd?: number;
  uid?: number;
  callbackMsg?: number;
  version?: number;
  notifyId: string;
  actions: NotifyAction[];
  data?: unknown;
};

function bannerFromBus(b: IslandNotifyBanner): MsgBanner {
  const text = (b.body || b.title || "").trim() || "通知";
  return {
    key: b.id,
    text,
    iconPng: b.iconPng ?? "",
    title: b.title,
    source: b.source,
    pluginId: b.pluginId,
    trayIconId: b.tray?.iconId,
    hwnd: b.tray?.hwnd,
    uid: b.tray?.uid,
    callbackMsg: b.tray?.callbackMsg,
    version: b.tray?.version,
    notifyId: b.id,
    actions: b.actions ?? [],
    data: b.data,
  };
}

async function screenLogicalWidth() {
  const mon = await currentMonitor();
  if (!mon) return 1920;
  return Math.round(mon.size.width / mon.scaleFactor);
}

/** 窗口高度跟随岛高；AppBar 始终折叠高度，不跟着展开变。 */
let cachedScreenW: number | null = null;

async function setBarHeight(islandH: number) {
  if (cachedScreenW == null) cachedScreenW = await screenLogicalWidth();
  const width = cachedScreenW;
  await getCurrentWindow().setSize(new LogicalSize(width, winHeight(islandH)));
  try {
    await invoke("float_overlay");
  } catch {
    /* noop outside tauri */
  }
}

function App() {
  const [expanded, setExpanded] = useState(false);
  const [trayOpen, setTrayOpen] = useState(false);
  const [statusMenuOpen, setStatusMenuOpen] = useState(false);
  const [material] = useState<Material>("none");
  const [ambient, setAmbient] = useState<Ambient>({ r: 42, g: 42, b: 46 });
  const [chromeLeft, setChromeLeft] = useState(() => chromeTokens({ r: 42, g: 42, b: 46 }));
  const [chromeCenter, setChromeCenter] = useState(() => chromeTokens({ r: 42, g: 42, b: 46 }));
  const [chromeRight, setChromeRight] = useState(() => chromeTokens({ r: 42, g: 42, b: 46 }));
  const [size, setSize] = useState<IslandSize>(ISLAND_COLLAPSED);
  const [pulling, setPulling] = useState(false);
  const pullingRef = useRef(false);
  /** 展开/收起/回弹动画中：顶角强制直角，只在完全静止胶囊时恢复圆顶 */
  const morphingRef = useRef(false);
  const [springing, setSpringing] = useState(false);
  /** 0=折叠条 1=弹窗全开；拖拽/展开过程中间值 */
  const [reveal, setReveal] = useState(0);
  const [islandPrefs, setIslandPrefsState] = useState<IslandPrefs>(() => getIslandPrefs());
  /** 插件面板尺寸：settings.panelWidth / panelHeight（缺省取 defaultSize） */
  const [shellPanelW, setShellPanelW] = useState(VIEW_W_DEFAULT);
  const shellPanelWRef = useRef(shellPanelW);
  const [shellPanelH, setShellPanelH] = useState(VIEW_H_DEFAULT);
  const shellPanelHRef = useRef(shellPanelH);
  /** 各 island.bar 插件最近一次 setBar（歌词可与天气并存，由 pick 竞选） */
  const [contentBars, setContentBars] = useState<Map<string, IslandBarState>>(
    () => new Map(),
  );
  /** 临时层：中转站等；有内容时盖住内容竞选 */
  const [overlayBar, setOverlayBar] = useState<IslandBarState | null>(null);
  const barOrder = mergeBarPriority(
    islandPrefs.barPriority,
    listBarResidentProviders().map((p) => p.id),
    islandPrefs.barResident,
  );
  const contentBar = pickIslandContentBar(contentBars, barOrder);
  const islandBar = overlayBar ?? contentBar;
  const [dropTarget, setDropTarget] = useState(false);
  const [panelOverride, setPanelOverride] = useState<string | null>(null);
  const panelOverrideRef = useRef<string | null>(null);
  const [dropPluginId, setDropPluginId] = useState<string | null>(() =>
    resolveIslandDropPluginId(),
  );
  const dropPluginIdRef = useRef(dropPluginId);
  /** 沉浸：黑底变透明，字色跟随顶栏对比度 */
  const [immersed, setImmersed] = useState(false);
  /**
   * 面板生命周期：仅在展开动画结束后为 true；收起一开始为 false。
   * 驱动 hub.panel.onEnter / onLeave（镜子等勿在折叠态开摄像头）。
   */
  const [panelActive, setPanelActive] = useState(false);
  /** 托盘闪动消息提示（岛内落下） */
  const [msgBanner, setMsgBanner] = useState<MsgBanner | null>(null);
  const gen = useRef(0);
  const busy = useRef(false);
  const expandedRef = useRef(expanded);
  const trayOpenRef = useRef(trayOpen);
  const pluginPopupOpenRef = useRef(false);
  const sizeRef = useRef(size);
  const revealRef = useRef(0);
  const immersedRef = useRef(false);
  const islandPrefsRef = useRef(islandPrefs);
  const msgBannerRef = useRef<MsgBanner | null>(null);
  const notifyRef = useRef<HTMLDivElement>(null);
  const swipe = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    dx: number;
    active: boolean;
    moved: boolean;
    dismissed: boolean;
  } | null>(null);
  const islandRef = useRef<HTMLDivElement>(null);
  const settingsAnchorRef = useRef<HTMLDivElement>(null);
  const pathRef = useRef<SVGPathElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const shapeLayerRef = useRef<HTMLDivElement>(null);
  const islandUiRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const lastWinH = useRef(winHeight(ISLAND_COLLAPSED.height));
  const idleTimer = useRef<number | null>(null);
  const drag = useRef<{
    pointerId: number;
    startY: number;
    lastY: number;
    moved: boolean;
    active: boolean;
    /** 窗口已拉高到展开高度，否则长高会被 overflow 裁成顶部黑矩形 */
    winReady: boolean;
  } | null>(null);
  expandedRef.current = expanded;
  trayOpenRef.current = trayOpen;
  immersedRef.current = immersed;
  dropPluginIdRef.current = dropPluginId;
  islandPrefsRef.current = islandPrefs;
  panelOverrideRef.current = panelOverride;
  shellPanelWRef.current = shellPanelW;
  shellPanelHRef.current = shellPanelH;
  // size / reveal 只由 paintDom 维护，避免重渲染把动画进度打回旧值

  useEffect(() => {
    const sync = () => {
      setDropPluginId(resolveIslandDropPluginId());
      const barOk = (pluginId: string) => {
        const rec = pluginRegistry.get(pluginId);
        return Boolean(
          rec?.enabled &&
            rec.manifest.slots?.["island.bar"] &&
            (rec.manifest.capabilities ?? []).includes("island.bar"),
        );
      };
      setContentBars((prev) => {
        let changed = false;
        const next = new Map(prev);
        for (const id of prev.keys()) {
          if (!barOk(id)) {
            next.delete(id);
            changed = true;
          }
        }
        return changed ? next : prev;
      });
      setOverlayBar((prev) => {
        if (!prev) return prev;
        return barOk(prev.pluginId) ? prev : null;
      });
      setPanelOverride((prev) => {
        if (!prev?.startsWith("plugin:")) return prev;
        const pid = prev.slice("plugin:".length);
        return pluginRegistry.get(pid)?.enabled ? prev : null;
      });
    };
    sync();
    return pluginRegistry.subscribe(sync);
  }, []);

  useEffect(() => {
    // 非临时插件不得留在 overlay（改常驻设置时清掉误占位）
    setOverlayBar((prev) => {
      if (!prev) return prev;
      const rec = pluginRegistry.get(prev.pluginId);
      if (rec?.manifest.slots?.["island.bar"]?.excludeFromBarResident) return prev;
      return null;
    });
  }, [islandPrefs.barResident]);

  function clearIdleTimer() {
    if (idleTimer.current != null) {
      window.clearTimeout(idleTimer.current);
      idleTimer.current = null;
    }
  }

  /** 有交互时退出沉浸，并重新计时 */
  function bumpIslandActivity() {
    clearIdleTimer();
    if (immersedRef.current) {
      immersedRef.current = false;
      setImmersed(false);
    }
  }

  function dismissMsgBanner() {
    const el = notifyRef.current;
    if (el) {
      el.style.transition = "";
      el.style.transform = "";
      el.style.opacity = "";
    }
    const id = msgBannerRef.current?.notifyId;
    msgBannerRef.current = null;
    setMsgBanner(null);
    if (id) islandNotifyBus.dismiss(id);
    else islandNotifyBus.dismiss();
    scheduleImmerse();
  }

  function fireNotifyAction(action: NotifyAction) {
    const banner = msgBannerRef.current;
    if (!banner || banner.source !== "plugin" || !banner.pluginId) {
      dismissMsgBanner();
      return;
    }
    const payload = {
      pluginId: banner.pluginId,
      notifyId: banner.notifyId,
      actionId: action.id,
      data: action.data !== undefined ? action.data : banner.data,
    };
    void emit("island-notify-action", payload).catch(console.error);
    dismissMsgBanner();
  }

  /** 托盘闪动 → 通知总线（常驻，ttl=0） */
  function showMsgBanner(att: TrayAttention) {
    const prefs = islandPrefsRef.current;
    if (!prefs.msgNotify) return;
    if (expandedRef.current || revealRef.current > 0.05) return;

    bumpIslandActivity();
    clearIdleTimer();

    const text = (prefs.msgNotifyText || "收到一条消息").trim() || "收到一条消息";
    const title = (att.tooltip || att.process || "").trim();
    islandNotifyBus.push({
      source: "tray",
      title: title || text,
      body: text,
      iconPng: att.icon_png_base64,
      urgency: "active",
      ttlMs: 0,
      tray: {
        iconId: att.id,
        hwnd: att.hwnd,
        uid: att.uid,
        callbackMsg: att.callback_msg,
        version: att.version ?? 0,
      },
    });
  }

  function scheduleImmerse() {
    clearIdleTimer();
    const prefs = islandPrefsRef.current;
    if (!prefs.autoImmerse) return;
    if (expandedRef.current || trayOpenRef.current) return;
    if (revealRef.current > 0.02) return;
    if (busy.current) return;
    if (msgBannerRef.current) return;
    // 中转站不阻断沉浸：只跟设置「自动沉浸」
    idleTimer.current = window.setTimeout(() => {
      idleTimer.current = null;
      const latest = islandPrefsRef.current;
      if (!latest.autoImmerse) return;
      if (expandedRef.current || trayOpenRef.current || busy.current) return;
      if (revealRef.current > 0.02) return;
      if (msgBannerRef.current) return;
      immersedRef.current = true;
      setImmersed(true);
    }, prefs.immerseIdleSec * 1000);
  }

  /** 直接改 DOM；动画中不走 React，避免 ambient 等重渲染把路径打回旧值 */
  function paintDom(next: IslandSize, nextReveal: number) {
    sizeRef.current = next;
    revealRef.current = nextReveal;
    // 非沉浸：顶角始终直角贴屏（gap=0）。沉浸态 path 透明，同样贴顶，避免进出沉浸/收拢时纵向跳动。
    const topSquare = 1;
    const gap = 0;
    const w = Math.max(28, next.width);
    const h = Math.max(28, next.height);
    const root = islandRef.current;
    if (root) {
      root.style.top = `${gap}px`;
      root.style.width = `${w}px`;
      root.style.height = `${h}px`;
      // BorderBeam 描边默认矩形；用底角半径贴合 SVG 岛形
      root.style.setProperty("--island-r-bot", `${islandBottomRadius(w, h)}px`);
    }
    const shape = shapeLayerRef.current;
    if (shape) {
      shape.style.width = `${w}px`;
      shape.style.height = `${h}px`;
    }
    const svg = svgRef.current;
    if (svg) {
      svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
      svg.setAttribute("width", String(w));
      svg.setAttribute("height", String(h));
    }
    const path = pathRef.current;
    if (path) {
      path.setAttribute("d", islandPath(w, h, topSquare));
    }
    const ui = islandUiRef.current;
    if (ui) {
      ui.style.width = `${w}px`;
      ui.style.minHeight = `${h}px`;
    }
    const panel = panelRef.current;
    if (panel) {
      const open = nextReveal > 0.12;
      panel.style.opacity = open ? String(Math.min(1, (nextReveal - 0.12) / 0.55)) : "0";
      panel.classList.toggle("is-open", open);
    }
  }

  /** 按帧插值：只刷 DOM（回弹等简单过渡） */
  function animateVisual(
    to: IslandSize,
    revealTo: number,
    ms: number,
    token: number,
  ): Promise<void> {
    const from = { ...sizeRef.current };
    const revealFrom = revealRef.current;
    const t0 = performance.now();
    morphingRef.current = true;
    return new Promise((resolve) => {
      const step = (now: number) => {
        if (token !== gen.current) {
          morphingRef.current = false;
          resolve();
          return;
        }
        const p = clamp01((now - t0) / Math.max(1, ms));
        const e = easeOutSmooth(p);
        paintDom(
          {
            width: lerp(from.width, to.width, e),
            height: lerp(from.height, to.height, e),
          },
          lerp(revealFrom, revealTo, e),
        );
        if (p < 1) {
          requestAnimationFrame(step);
        } else {
          morphingRef.current = false;
          paintDom(to, revealTo);
          setSize(to);
          setReveal(revealTo);
          resolve();
        }
      };
      requestAnimationFrame(step);
    });
  }

  /**
   * 打开 / 收回：
   * - 打开：先变宽，再变高（宽 0→0.55，高 0.08→1）
   * - 收回：对称反过来——先变窄（保持高度），再变矮，避免顶部留下不跟手的宽扁黑块
   */
  function animateMorph(token: number, opening: boolean): Promise<void> {
    const t0 = performance.now();
    morphingRef.current = true;
    paintDom(sizeRef.current, revealRef.current);
    return new Promise((resolve) => {
      const step = (now: number) => {
        if (token !== gen.current) {
          morphingRef.current = false;
          resolve();
          return;
        }
        const p = clamp01((now - t0) / MORPH_MS);
        let wE: number;
        let hE: number;
        let rE: number;
        if (opening) {
          wE = channelEase(p, 0, 0.55);
          hE = channelEase(p, 0.12, 1);
          rE = channelEase(p, 0.2, 1);
        } else {
          // 先收窄：p=0→0.55 宽度 1→0；再收矮：p=0.2→1 高度 1→0
          wE = 1 - channelEase(p, 0, 0.55);
          hE = 1 - channelEase(p, 0.2, 1);
          rE = 1 - channelEase(p, 0, 0.35);
        }
        paintDom(
          {
            width: lerp(ISLAND_COLLAPSED.width, liveExpanded.width, wE),
            height: lerp(ISLAND_COLLAPSED.height, liveExpanded.height, hE),
          },
          rE,
        );
        if (p < 1) {
          requestAnimationFrame(step);
        } else {
          const end = opening ? { ...liveExpanded } : ISLAND_COLLAPSED;
          const endReveal = opening ? 1 : 0;
          if (opening) morphingRef.current = false;
          paintDom(end, endReveal);
          setSize(end);
          setReveal(endReveal);
          resolve();
        }
      };
      requestAnimationFrame(step);
    });
  }

  /** 点击展开：宽高交错长大，不经过宽扁中间态 */
  async function expand() {
    if (busy.current || expandedRef.current) return;
    bumpIslandActivity();
    // 展开下拉面板前先关窗口组等托管弹窗（快捷区 click 常带 blur suppress，失焦关会失效）
    dismissPluginPopup();
    const token = ++gen.current;
    busy.current = true;
    trayOpenRef.current = false;
    setTrayOpen(false);
    pullingRef.current = false;
    setPulling(false);
    setSpringing(false);
    try {
      // 点击展开：立刻直角贴顶
      morphingRef.current = true;
      paintDom(sizeRef.current, revealRef.current);
      await setBarHeight(liveExpanded.height);
      lastWinH.current = winHeight(liveExpanded.height);
      if (token !== gen.current) return;
      setExpanded(true);
      await animateMorph(token, true);
      if (token !== gen.current) return;
      setPanelActive(true);
    } finally {
      if (token === gen.current) {
        busy.current = false;
        // 若展开过程中目标尺寸已切到中转站，收尾再贴合一次
        if (expandedRef.current) {
          const t = { width: liveExpanded.width, height: liveExpanded.height };
          if (
            Math.abs(sizeRef.current.width - t.width) > 1 ||
            Math.abs(sizeRef.current.height - t.height) > 1
          ) {
            paintDom(t, 1);
            setSize(t);
            void setBarHeight(t.height);
            lastWinH.current = winHeight(t.height);
          }
        }
      }
    }
  }

  /** 收起：一条时间线交错收高度与宽度 */
  async function collapse() {
    if (!expandedRef.current && revealRef.current <= 0.01) return;
    bumpIslandActivity();
    const token = ++gen.current;
    busy.current = true;
    try {
      // 先 leave：立刻关摄像头，再开收起动画
      setPanelActive(false);
      setExpanded(false);
      // 收回一开始就直角贴顶，与下拉同理
      morphingRef.current = true;
      paintDom(sizeRef.current, revealRef.current);
      await animateMorph(token, false);
      if (token !== gen.current) return;
      await setBarHeight(ISLAND_COLLAPSED.height);
      lastWinH.current = winHeight(ISLAND_COLLAPSED.height);
      morphingRef.current = false;
      paintDom(ISLAND_COLLAPSED, 0);
      // 拖入会话覆盖仅本次展开有效；收起后恢复用户「下拉内容」
      setPanelOverride(null);
    } finally {
      if (token === gen.current) {
        morphingRef.current = false;
        busy.current = false;
        scheduleImmerse();
      }
    }
  }

  /** 拉高悬浮窗到展开高度（AppBar 高度不变）；拖拽前预热，避免裁切黑块 */
  function ensureExpandedWindow(): Promise<void> {
    if (lastWinH.current >= winHeight(liveExpanded.height)) {
      return Promise.resolve();
    }
    lastWinH.current = winHeight(liveExpanded.height);
    return setBarHeight(liveExpanded.height);
  }

  function onIslandPointerDown(e: ReactPointerEvent<HTMLDivElement>) {
    if (expandedRef.current || busy.current) return;
    if (e.button !== 0) return;
    // 消息提示：支持左滑划掉；点击仍打开应用
    if (msgBannerRef.current) {
      e.currentTarget.setPointerCapture(e.pointerId);
      swipe.current = {
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
        dx: 0,
        active: true,
        moved: false,
        dismissed: false,
      };
      bumpIslandActivity();
      return;
    }
    bumpIslandActivity();
    e.currentTarget.setPointerCapture(e.pointerId);
    const needRaise = lastWinH.current < winHeight(liveExpanded.height);
    drag.current = {
      pointerId: e.pointerId,
      startY: e.clientY,
      lastY: e.clientY,
      moved: false,
      active: true,
      winReady: !needRaise,
    };
    setSpringing(false);
    pullingRef.current = true;
    setPulling(true);
    // 按下瞬间就把 SVG 顶角改成直角贴顶（与手势同帧）
    paintDom(sizeRef.current, revealRef.current);
    // 必须先拉高窗口再长高岛形；否则 #root overflow 会把胶囊裁成顶部黑矩形，跟手滞后
    if (needRaise) {
      void ensureExpandedWindow().then(() => {
        const d = drag.current;
        if (!d || d.pointerId !== e.pointerId) return;
        d.winReady = true;
        // 以当前指位重新锚定，避免等待期间的位移一次性捅出去
        d.startY = d.lastY;
      });
    }
  }

  function onIslandPointerMove(e: ReactPointerEvent<HTMLDivElement>) {
    const s = swipe.current;
    if (s?.active && s.pointerId === e.pointerId) {
      const dx = e.clientX - s.startX;
      const dy = e.clientY - s.startY;
      if (Math.abs(dx) > CLICK_SLOP || Math.abs(dy) > CLICK_SLOP) s.moved = true;
      // 以横向左滑为主
      if (dx < 0 && Math.abs(dx) >= Math.abs(dy)) {
        s.dx = dx;
        const el = notifyRef.current;
        if (el) {
          el.classList.add("is-swiping");
          el.style.transition = "none";
          el.style.transform = `translateX(${dx}px)`;
          el.style.opacity = String(Math.max(0.15, 1 + dx / 140));
        }
      }
      return;
    }
    const d = drag.current;
    if (!d?.active || d.pointerId !== e.pointerId) return;
    d.lastY = e.clientY;
    // 窗口未就绪时只记位置，不拉高外形（避免裁切黑块）
    if (!d.winReady) return;
    const raw = e.clientY - d.startY;
    if (Math.abs(raw) > CLICK_SLOP) d.moved = true;
    const p = pullProgress(raw);
    paintDom(sizeFromProgress(p), p);
  }

  function finishPull(open: boolean) {
    drag.current = null;
    pullingRef.current = false;
    setPulling(false);
    if (open) {
      dismissPluginPopup();
      setSpringing(false);
      void (async () => {
        busy.current = true;
        const token = ++gen.current;
        try {
          await setBarHeight(liveExpanded.height);
          lastWinH.current = winHeight(liveExpanded.height);
          if (token !== gen.current) return;
          setExpanded(true);
          await animateVisual({ ...liveExpanded }, 1, HEIGHT_MS, token);
          if (token !== gen.current) return;
          setPanelActive(true);
        } finally {
          if (token === gen.current) busy.current = false;
        }
      })();
      return;
    }
    setSpringing(true);
    setPanelActive(false);
    void (async () => {
      const token = ++gen.current;
      morphingRef.current = true;
      paintDom(sizeRef.current, revealRef.current);
      if (token !== gen.current) return;
      // 未拉满：从当前尺寸收回（不走完整倒放，避免跳变）
      await animateVisual(ISLAND_COLLAPSED, 0, SPRING_MS, token);
      if (token !== gen.current) return;
      setSpringing(false);
      if (!expandedRef.current && !trayOpenRef.current) {
        await setBarHeight(ISLAND_COLLAPSED.height);
        lastWinH.current = winHeight(ISLAND_COLLAPSED.height);
      }
      morphingRef.current = false;
      paintDom(ISLAND_COLLAPSED, 0);
      setPanelOverride(null);
      scheduleImmerse();
    })();
  }

  function onIslandPointerUp(e: ReactPointerEvent<HTMLDivElement>) {
    const s = swipe.current;
    if (s?.active && s.pointerId === e.pointerId) {
      s.active = false;
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        /* noop */
      }
      const dx = s.dx;
      const el = notifyRef.current;
      if (dx < -56) {
        s.dismissed = true;
        if (el) {
          el.classList.add("is-swiping");
          el.style.transition = "transform 220ms ease, opacity 200ms ease";
          el.style.transform = "translateX(-120%)";
          el.style.opacity = "0";
        }
        window.setTimeout(() => dismissMsgBanner(), 200);
        window.setTimeout(() => {
          swipe.current = null;
        }, 280);
      } else {
        if (el) {
          el.style.transition =
            "transform 220ms cubic-bezier(0.22, 1, 0.36, 1), opacity 180ms ease";
          el.style.transform = "";
          el.style.opacity = "";
          window.setTimeout(() => el.classList.remove("is-swiping"), 220);
        }
        // 有明显滑动则吞掉 click；轻点仍可开应用
        if (s.moved && Math.abs(dx) > CLICK_SLOP) {
          s.dismissed = true;
          window.setTimeout(() => {
            swipe.current = null;
          }, 280);
        } else {
          swipe.current = null;
        }
      }
      return;
    }

    const d = drag.current;
    if (!d?.active || d.pointerId !== e.pointerId) return;
    d.active = false;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* noop */
    }
    if (!d.moved) {
      drag.current = null;
      pullingRef.current = false;
      setPulling(false);
      setSpringing(false);
      setSize({ ...sizeRef.current });
      setReveal(revealRef.current);
      void expand();
      return;
    }
    setSize({ ...sizeRef.current });
    setReveal(revealRef.current);
    finishPull(revealRef.current >= PULL_OPEN);
  }

  function onIslandPointerCancel(e: ReactPointerEvent<HTMLDivElement>) {
    const s = swipe.current;
    if (s?.active && s.pointerId === e.pointerId) {
      s.active = false;
      const el = notifyRef.current;
      if (el) {
        el.style.transition = "transform 220ms ease, opacity 180ms ease";
        el.style.transform = "";
        el.style.opacity = "";
      }
      swipe.current = null;
      return;
    }
    const d = drag.current;
    if (!d?.active || d.pointerId !== e.pointerId) return;
    d.active = false;
    finishPull(false);
  }

  function onIslandKeyDown(e: ReactKeyboardEvent<HTMLDivElement>) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (expandedRef.current) void collapse();
      else void expand();
    }
  }

  useEffect(() => {
    void (async () => {
      await setBarHeight(ISLAND_COLLAPSED.height);
      lastWinH.current = winHeight(ISLAND_COLLAPSED.height);
      try {
        await invoke("set_window_material", { material: "none" });
      } catch {
        /* noop */
      }
    })();
  }, []);

  useLayoutEffect(() => {
    paintDom(sizeRef.current, revealRef.current);
  }, []);

  useEffect(() => {
    if (expanded || busy.current || pulling || springing) return;
    // 托盘改为独立弹窗，主顶栏保持折叠高度
    if (!trayOpen) void setBarHeight(ISLAND_COLLAPSED.height);
  }, [trayOpen, expanded, pulling, springing]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged((ev) => {
        if (!ev.payload) {
          // 托盘是独立窗口，会抢走主窗焦点；勿在此关托盘（由 tray-popup 失焦自行关闭）
          if (expandedRef.current || revealRef.current > 0.01) {
            trayOpenRef.current = false;
            setTrayOpen(false);
            void collapse();
          }
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Track plugin-popup visibility for click-outside close on the island bar. */
  useEffect(() => {
    const unsubs: Array<() => void> = [];
    void listen("plugin-popup-opened", () => {
      pluginPopupOpenRef.current = true;
      // 弹窗与岛面板互斥：开窗口组等弹窗时收起下拉面板
      if (expandedRef.current) {
        setPanelOverride(null);
        void collapse();
      }
    }).then((fn) => unsubs.push(fn));
    void listen("plugin-popup-closed", () => {
      pluginPopupOpenRef.current = false;
    }).then((fn) => unsubs.push(fn));
    void invoke<boolean>("is_plugin_popup_open")
      .then((open) => {
        pluginPopupOpenRef.current = !!open;
      })
      .catch(() => undefined);
    return () => unsubs.forEach((fn) => fn());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 打开岛面板 / 展开岛时关掉托管弹窗（窗口组等），避免 always-on-top 残留。 */
  function dismissPluginPopup() {
    pluginPopupOpenRef.current = false;
    void invoke("close_plugin_popup").catch(() => undefined);
  }

  function closePluginPopupFromIsland(e?: { target?: EventTarget | null }) {
    if (!pluginPopupOpenRef.current) return;
    const t = e?.target as Element | null | undefined;
    // Let shortcuts / tray / status menu handle their own toggle.
    if (
      t?.closest?.(
        ".shortcuts-host, .shortcuts-plugin-strip, .settings-anchor, .tray-cluster, .tray-root",
      )
    ) {
      return;
    }
    dismissPluginPopup();
  }

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let chromeTimer: number | undefined;
    let lastRgb = "";

    const frostRef = { current: getIslandPrefs().topbarFrost };

    const applyStripDom = (a: Ambient, frost = frostRef.current) => {
      const el = document.querySelector(".ambient-strip") as HTMLElement | null;
      if (!el) return;
      el.style.opacity = "1";
      if (frost) {
        el.style.backgroundColor = `rgba(${a.r}, ${a.g}, ${a.b}, 0.72)`;
      } else {
        el.style.backgroundColor = `rgb(${a.r}, ${a.g}, ${a.b})`;
      }
      if (a.png_base64 && (a.width ?? 0) > 1) {
        if (frost) {
          el.style.backgroundImage = `linear-gradient(rgba(255,255,255,0.06), rgba(255,255,255,0)), url(data:image/png;base64,${a.png_base64})`;
          el.style.backgroundRepeat = "no-repeat, no-repeat";
          el.style.backgroundSize =
            a.offset_x === 0
              ? "100% 100%, 100% 100%"
              : a.span_width && a.span_width > 0
                ? `100% 100%, ${a.span_width}px 100%`
                : "100% 100%, 100% 100%";
          el.style.backgroundPosition =
            a.offset_x === 0
              ? "0 0, 0 0"
              : typeof a.offset_x === "number"
                ? `0 0, ${a.offset_x}px 0`
                : "0 0, 0 0";
          el.style.opacity = "0.78";
        } else {
          el.style.backgroundImage = `url(data:image/png;base64,${a.png_base64})`;
          el.style.backgroundRepeat = "no-repeat";
          el.style.backgroundSize =
            a.offset_x === 0
              ? "100% 100%"
              : a.span_width && a.span_width > 0
                ? `${a.span_width}px 100%`
                : "100% 100%";
          el.style.backgroundPosition =
            a.offset_x === 0
              ? "0 0"
              : typeof a.offset_x === "number"
                ? `${a.offset_x}px 0`
                : "0 0";
        }
      } else {
        el.style.backgroundImage = "none";
      }
    };

    const scheduleChrome = (a: Ambient) => {
      if (chromeTimer != null) window.clearTimeout(chromeTimer);
      chromeTimer = window.setTimeout(() => {
        void (async () => {
          let left = { r: a.r, g: a.g, b: a.b };
          let center = left;
          let right = left;
          if (a.png_base64 && (a.width ?? 0) > 1) {
            const bands = await sampleStripBands(a.png_base64);
            if (bands) {
              left = bands.left;
              center = bands.center;
              right = bands.right;
            }
          }
          if (cancelled) return;
          setChromeLeft(chromeTokens(left));
          setChromeCenter(chromeTokens(center));
          setChromeRight(chromeTokens(right));
        })();
      }, 80);
    };

    void (async () => {
      try {
        const first = await invoke<Ambient>("sample_ambient_color");
        if (cancelled) return;
        applyStripDom(first);
        lastRgb = `${first.r},${first.g},${first.b}`;
        setAmbient({ r: first.r, g: first.g, b: first.b });
        scheduleChrome(first);
      } catch {
        /* noop */
      }
      try {
        unlisten = await listen<Ambient>("ambient-color", (ev) => {
          const a = ev.payload;
          frostRef.current = getIslandPrefs().topbarFrost;
          applyStripDom(a);
          const rgb = `${a.r},${a.g},${a.b}`;
          if (rgb !== lastRgb) {
            lastRgb = rgb;
            // Keep React state lean — no giant png_base64 in state.
            setAmbient({ r: a.r, g: a.g, b: a.b });
          }
          scheduleChrome(a);
        });
      } catch {
        /* noop */
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
      if (chromeTimer != null) window.clearTimeout(chromeTimer);
    };
  }, []);

  /** 当前会话 / 投放插件：同步面板壳尺寸（defaultSize 或 staging settings） */
  const sizePluginId =
    parsePluginPanelId(panelOverride ?? islandPrefs.pullContent) ?? dropPluginId;

  useEffect(() => {
    if (!sizePluginId) return;
    let cancelled = false;
    const apply = (settings?: Record<string, unknown> | null) => {
      const { w, h } = resolvePluginPanelShellSize(sizePluginId, settings, {
        w: clampStagingPanelW,
        h: clampStagingPanelH,
      });
      if (cancelled) return;
      shellPanelWRef.current = w;
      shellPanelHRef.current = h;
      setShellPanelW(w);
      setShellPanelH(h);
      liveExpanded.width = w;
      liveExpanded.height = h;
    };
    // Sync from manifest first so expand never uses stale staging 560×152
    apply(null);
    void invoke<Record<string, unknown>>("hub_settings_get_all", {
      pluginId: sizePluginId,
    })
      .then((s) => apply(s))
      .catch(() => undefined);
    let unlisten: (() => void) | undefined;
    void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
      "plugin-settings-changed",
      (ev) => {
        if (ev.payload?.pluginId !== sizePluginId) return;
        apply(ev.payload.settings ?? null);
      },
    ).then((fn) => {
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sizePluginId]);

  useEffect(() => {
    void hydrateIslandPrefs().then(setIslandPrefsState);
    const unsub = subscribeIslandPrefs(setIslandPrefsState);
    const unsubBus = islandNotifyBus.subscribe((b) => {
      if (!b) {
        msgBannerRef.current = null;
        setMsgBanner(null);
        return;
      }
      if (expandedRef.current || revealRef.current > 0.05) return;
      bumpIslandActivity();
      clearIdleTimer();
      const next = bannerFromBus(b);
      msgBannerRef.current = next;
      setMsgBanner(next);
    });
    let unlistenPrefs: (() => void) | undefined;
    let unlistenAttn: (() => void) | undefined;
    let unlistenPluginNotify: (() => void) | undefined;
    let unlistenStaging: (() => void) | undefined;
    let unlistenBar: (() => void) | undefined;
    let unlistenSession: (() => void) | undefined;
    let unsubPlugins = () => {};
    void bootstrapPlugins();
    void subscribeInstalledPlugins().then((fn) => {
      unsubPlugins = fn;
    });
    void listen<IslandPrefs>("island-prefs", (ev) => {
      setIslandPrefsState(applyIslandPrefsSnapshot(ev.payload));
    }).then((fn) => {
      unlistenPrefs = fn;
    });
    const applyStagingBar = (
      pluginId: string,
      summary: { files: number; texts: number; images: number; total: number },
    ) => {
      const rec = pluginRegistry.get(pluginId);
      if (
        !rec?.enabled ||
        !rec.manifest.slots?.["island.bar"] ||
        !(rec.manifest.capabilities ?? []).includes("island.bar")
      ) {
        return;
      }
      if (summary.total <= 0) {
        setOverlayBar((prev) => (prev?.pluginId === pluginId ? null : prev));
        setPanelOverride((prev) => (prev === `plugin:${pluginId}` ? null : prev));
        return;
      }
      setOverlayBar({
        pluginId,
        text: formatStagingBarText(rec.manifest.name || pluginId, summary),
        title: rec.manifest.name || pluginId,
      });
    };
    const dropId = resolveIslandDropPluginId();
    if (dropId) {
      void invoke<{ files: number; texts: number; images: number; total: number }>(
        "hub_staging_summary",
        { pluginId: dropId },
      )
        .then((s) => applyStagingBar(dropId, s))
        .catch(() => undefined);
    }
    /** Host 通用：staging + island.bar → 岛栏文案；无 panel 时也能显示 */
    void listen("staging-changed", (ev) => {
      const { pluginId, summary } = normalizeStagingChanged(
        ev.payload as Parameters<typeof normalizeStagingChanged>[0],
      );
      if (!pluginId) return;
      applyStagingBar(pluginId, summary);
    }).then((fn) => {
      unlistenStaging = fn;
    });
    void listen<IslandBarState | null>("island-bar-changed", (ev) => {
      const p = ev.payload;
      if (!p?.pluginId) return;
      const cleared = !String(p.text ?? "").trim();
      const next = cleared
        ? null
        : { pluginId: p.pluginId, text: p.text, title: p.title };
      const rec = pluginRegistry.get(p.pluginId);
      if (
        !rec?.enabled ||
        !rec.manifest.slots?.["island.bar"] ||
        !(rec.manifest.capabilities ?? []).includes("island.bar")
      ) {
        return;
      }
      const tempOnly = Boolean(rec.manifest.slots?.["island.bar"]?.excludeFromBarResident);
      // 临时层：中转站等（盖住歌词/天气）
      if (tempOnly) {
        setOverlayBar((prev) => {
          if (cleared) return prev?.pluginId === p.pluginId ? null : prev;
          return next;
        });
        return;
      }
      // 内容层：所有非临时 island.bar 可写；展示顺序由 settings.barPriority 决定
      setContentBars((prev) => {
        const cur = prev.get(p.pluginId);
        if (cleared) {
          if (!cur) return prev;
          const m = new Map(prev);
          m.delete(p.pluginId);
          return m;
        }
        if (
          cur &&
          cur.text === next!.text &&
          (cur.title ?? "") === (next!.title ?? "")
        ) {
          return prev;
        }
        const m = new Map(prev);
        m.set(p.pluginId, next!);
        return m;
      });
    }).then((fn) => {
      unlistenBar = fn;
    });
    void listen<{ action?: string; pluginId?: string }>("island-session", (ev) => {
      const action = ev.payload?.action;
      const pluginId = ev.payload?.pluginId;
      if (action === "open" && pluginId) {
        // Same plugin panel already expanded → collapse (second click closes).
        const cur =
          parsePluginPanelId(panelOverrideRef.current ?? islandPrefsRef.current.pullContent) ??
          null;
        if (expandedRef.current && cur === pluginId) {
          setPanelOverride(null);
          void collapse();
          return;
        }
        // 开岛面板时关掉窗口组等弹窗（与 expand 双保险；已展开时 expand 会 early-return）
        dismissPluginPopup();
        armPluginSession(pluginId);
        if (!expandedRef.current) void expand();
      } else if (action === "close") {
        setPanelOverride(null);
        if (expandedRef.current) void collapse();
      }
    }).then((fn) => {
      unlistenSession = fn;
    });
    void listen<TrayAttention>("tray-attention", (ev) => {
      showMsgBanner(ev.payload);
    }).then((fn) => {
      unlistenAttn = fn;
    });
    void listen<{
      pluginId: string;
      maxPerMinute?: number;
      title: string;
      body?: string;
      iconPng?: string;
      urgency?: "passive" | "active" | "critical";
      ttlMs?: number;
      actions?: unknown;
      data?: unknown;
    }>("island-notify", (ev) => {
      const p = ev.payload;
      const prefs = islandPrefsRef.current;
      islandNotifyBus.push(
        {
          source: "plugin",
          pluginId: p.pluginId,
          title: p.title,
          body: p.body,
          iconPng: p.iconPng,
          urgency: p.urgency ?? "active",
          ttlMs: p.ttlMs ?? prefs.msgNotifySec * 1000,
          actions: normalizeNotifyActions(p.actions),
          data: p.data,
        },
        p.maxPerMinute,
      );
    }).then((fn) => {
      unlistenPluginNotify = fn;
    });
    return () => {
      unsub();
      unsubBus();
      unsubPlugins();
      unlistenPrefs?.();
      unlistenAttn?.();
      unlistenPluginNotify?.();
      unlistenStaging?.();
      unlistenBar?.();
      unlistenSession?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    // 展开 / 托盘 / 拖放高亮 / 通知：暂停沉浸。中转站有内容不阻断。
    if (expanded || trayOpen || pulling || springing || reveal > 0.02 || msgBanner || dropTarget) {
      clearIdleTimer();
      if (immersedRef.current) {
        immersedRef.current = false;
        setImmersed(false);
      }
      return;
    }
    if (!islandPrefs.autoImmerse) {
      clearIdleTimer();
      if (immersedRef.current) {
        immersedRef.current = false;
        setImmersed(false);
      }
      return;
    }
    if (!immersed) scheduleImmerse();
    return () => clearIdleTimer();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    expanded,
    trayOpen,
    pulling,
    springing,
    reveal,
    msgBanner,
    dropTarget,
    islandPrefs.autoImmerse,
    islandPrefs.immerseIdleSec,
    immersed,
  ]);

  const effectivePullContent = panelOverride ?? islandPrefs.pullContent;
  const activePanelPluginId = parsePluginPanelId(effectivePullContent);
  const stagingBar = islandBar?.text ?? "";
  const viewW = activePanelPluginId ? shellPanelW : VIEW_W_DEFAULT;
  const viewH = activePanelPluginId ? shellPanelH : VIEW_H_DEFAULT;
  const pluginStagingShell =
    !!activePanelPluginId && isStagingPanelShell(viewW, viewH);
  liveExpanded.width = viewW;
  liveExpanded.height = viewH;

  /** 展开态下目标尺寸变化时做宽高插值（天气 ↔ 中转站） */
  function morphExpandedSize(target: IslandSize) {
    if (
      Math.abs(sizeRef.current.width - target.width) < 1 &&
      Math.abs(sizeRef.current.height - target.height) < 1
    ) {
      return;
    }
    const token = ++gen.current;
    busy.current = true;
    void (async () => {
      try {
        const maxH = Math.max(sizeRef.current.height, target.height);
        if (lastWinH.current < winHeight(maxH)) {
          await setBarHeight(maxH);
          lastWinH.current = winHeight(maxH);
        }
        if (token !== gen.current) return;
        await animateVisual(target, 1, HEIGHT_MS, token);
        if (token !== gen.current) return;
        await setBarHeight(target.height);
        lastWinH.current = winHeight(target.height);
        setSize(target);
        setReveal(1);
        paintDom(target, 1);
      } finally {
        if (token === gen.current) busy.current = false;
      }
    })();
  }

  // 展开中且目标尺寸变化：大岛↔小岛都走动画（清空中转站 / 打开中转站）
  // 跳过正在 expand/collapse 的帧，避免 ++gen 打断开合 morph
  useLayoutEffect(() => {
    liveExpanded.width = viewW;
    liveExpanded.height = viewH;
    if (!expanded) return;
    if (busy.current || morphingRef.current) return;
    morphExpandedSize({ width: viewW, height: viewH });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activePanelPluginId, viewW, viewH, expanded]);

  /** 打开会话面板前同步 liveExpanded 为目标插件尺寸（同步读 defaultSize，避免直开时仍用中转站 560×152） */
  function armPluginSession(pluginId: string) {
    const { w, h } = resolvePluginPanelShellSize(pluginId, null, {
      w: clampStagingPanelW,
      h: clampStagingPanelH,
    });
    shellPanelWRef.current = w;
    shellPanelHRef.current = h;
    setShellPanelW(w);
    setShellPanelH(h);
    liveExpanded.width = w;
    liveExpanded.height = h;
    setPanelOverride(`plugin:${pluginId}`);
  }

  async function openPluginSession(pluginId: string | null | undefined) {
    if (!pluginId || !pluginRegistry.get(pluginId)?.enabled) return;
    dismissPluginPopup();
    armPluginSession(pluginId);
    if (!expandedRef.current) {
      void expand();
    }
  }

  async function ingestDrop(dt: DataTransfer | null) {
    const pluginId = dropPluginIdRef.current;
    if (!pluginId || !dt) return;
    const text = dt.getData("text/plain");
    if (text && text.trim()) {
      await invoke("hub_staging_add_text", { pluginId, text }).catch(console.error);
    }
    const files = dt.files;
    if (files?.length) {
      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        const f = files.item(i);
        if (!f) continue;
        // WebView2 / Chromium：本地文件常带 path，与 onDragDropEvent 双通道
        const path = (f as File & { path?: string }).path;
        if (typeof path === "string" && path.trim()) {
          paths.push(path);
          continue;
        }
        if (!f.type.startsWith("image/")) continue;
        try {
          const buf = new Uint8Array(await f.arrayBuffer());
          const ext = (f.name.split(".").pop() || "png").toLowerCase();
          await invoke("hub_staging_add_image_bytes", {
            pluginId,
            label: f.name || "图片",
            bytes: Array.from(buf),
            ext,
          });
        } catch (err) {
          console.error(err);
        }
      }
      if (paths.length) {
        await invoke("hub_staging_add_paths", { pluginId, paths }).catch(console.error);
      }
    }
  }

  function onIslandDragEnter(e: ReactDragEvent) {
    if (!dropPluginIdRef.current) return;
    e.preventDefault();
    e.stopPropagation();
    bumpIslandActivity();
    setDropTarget(true);
  }

  function onIslandDragOver(e: ReactDragEvent) {
    if (!dropPluginIdRef.current) return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
    if (!dropTarget) setDropTarget(true);
  }

  function onIslandDragLeave(e: ReactDragEvent) {
    if (!dropPluginIdRef.current) return;
    e.preventDefault();
    const related = e.relatedTarget as Node | null;
    if (related && e.currentTarget.contains(related)) return;
    setDropTarget(false);
  }

  async function onIslandDrop(e: ReactDragEvent) {
    const pluginId = dropPluginIdRef.current;
    if (!pluginId) return;
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(false);
    bumpIslandActivity();
    await ingestDrop(e.dataTransfer);
    await openPluginSession(pluginId);
  }

  useEffect(() => {
    let un: (() => void) | undefined;
    void getCurrentWindow()
      .onDragDropEvent((ev) => {
        if (!dropPluginIdRef.current) {
          if (ev.payload.type === "leave" || ev.payload.type === "drop") {
            setDropTarget(false);
          }
          return;
        }
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") {
          bumpIslandActivity();
          setDropTarget(true);
        } else if (p.type === "leave") {
          setDropTarget(false);
        } else if (p.type === "drop") {
          setDropTarget(false);
          bumpIslandActivity();
          const pluginId = dropPluginIdRef.current;
          const paths = p.paths ?? [];
          if (pluginId && paths.length) {
            void invoke("hub_staging_add_paths", { pluginId, paths })
              .then(() => openPluginSession(pluginId))
              .catch(console.error);
          } else if (pluginId) {
            void openPluginSession(pluginId);
          }
        }
      })
      .then((fn) => {
        un = fn;
      })
      .catch(() => undefined);
    return () => un?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 通知快捷区 iframe 同步 --wh-chrome-fg（CSS 变量本身进不了跨文档 iframe）
  useEffect(() => {
    window.dispatchEvent(
      new CustomEvent("wh-chrome-changed", {
        detail: {
          left: chromeLeft.scheme,
          center: chromeCenter.scheme,
          right: chromeRight.scheme,
          fg: chromeLeft.fg,
        },
      }),
    );
  }, [chromeLeft, chromeCenter, chromeRight]);

  const ambientCss = {
    ["--island-top-gap" as string]: `${TOP_GAP}px`,
    ["--ambient-r" as string]: String(ambient.r),
    ["--ambient-g" as string]: String(ambient.g),
    ["--ambient-b" as string]: String(ambient.b),
    ...chromeCssVars("left", chromeLeft),
    ...chromeCssVars("center", chromeCenter),
    ...chromeCssVars("right", chromeRight),
  } as CSSProperties;

  const stripStyle: CSSProperties = {
    // PNG band is applied via DOM in ambient listener (avoids React re-render thrash).
    backgroundColor: islandPrefs.topbarFrost
      ? `rgba(${ambient.r}, ${ambient.g}, ${ambient.b}, 0.72)`
      : `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})`,
  };

  const shellExpanded = expanded || reveal > 0.2;

  return (
    <div
      className={`shell${shellExpanded ? " is-expanded" : ""}${islandPrefs.topbarFrost ? " is-topbar-frost" : ""}`}
      style={ambientCss}
      data-material={material}
      data-chrome-left={chromeLeft.scheme}
      data-chrome-right={chromeRight.scheme}
      onPointerDownCapture={(e) => closePluginPopupFromIsland(e)}
    >
      <div className="ambient-strip" style={stripStyle} aria-hidden />

      {expanded && (
        <div
          className="dismiss-backdrop"
          aria-hidden
          onPointerDown={(e) => {
            // 点岛左右空白：关掉弹窗（透明区原先会被系统点透）
            e.preventDefault();
            trayOpenRef.current = false;
            setTrayOpen(false);
            pluginPopupOpenRef.current = false;
            void invoke("close_plugin_popup").catch(() => undefined);
            if (expandedRef.current || revealRef.current > 0.01) {
              void collapse();
            }
          }}
        />
      )}

      <div
        ref={settingsAnchorRef}
        className="settings-anchor"
        onClick={(e) => e.stopPropagation()}
      >
        <StatusMenu
          anchorRef={settingsAnchorRef}
          menuOpen={statusMenuOpen}
          onMenuOpenChange={setStatusMenuOpen}
        />
      </div>

      <ShortcutsHost settingsRef={settingsAnchorRef} islandWidth={size.width} />

      <TrayCluster open={trayOpen} onOpenChange={setTrayOpen} />

      <BorderBeam
        ref={islandRef}
        size="pulse-inner"
        colorVariant="colorful"
        strength={0.7}
        borderRadius={Math.round(islandBottomRadius(size.width, size.height))}
        active={!!msgBanner}
        className="island-beam"
        style={
          {
            overflow: "visible",
            ["--island-r-bot"]: `${islandBottomRadius(size.width, size.height)}px`,
          } as CSSProperties
        }
      >
        <div
          className={`island-root${expanded ? " is-expanded" : ""}${pulling ? " is-pulling" : ""}${springing ? " is-springing" : ""}${immersed ? " is-immersed" : ""}${msgBanner ? " is-notifying" : ""}${dropTarget ? " is-drop-target" : ""}${islandBar ? " has-staging" : ""}`}
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          aria-label={expanded ? "收起灵动岛" : "下拉或点击展开灵动岛"}
          data-chrome={
            dropTarget
              ? "dark"
              : islandPrefs.topbarFrost || immersed
                ? chromeCenter.scheme
                : "dark"
          }
          onDragEnter={onIslandDragEnter}
          onDragOver={onIslandDragOver}
          onDragLeave={onIslandDragLeave}
          onDrop={(e) => void onIslandDrop(e)}
          onPointerDown={onIslandPointerDown}
          onPointerMove={onIslandPointerMove}
          onPointerUp={onIslandPointerUp}
          onPointerCancel={onIslandPointerCancel}
          onPointerEnter={() => {
            // 悬停时预拉高窗口，按下拖动即可立刻跟手
            if (!expandedRef.current && !busy.current) void ensureExpandedWindow();
          }}
          onPointerLeave={() => {
            if (drag.current?.active || expandedRef.current || busy.current) return;
            if (revealRef.current > 0.01) return;
            lastWinH.current = winHeight(ISLAND_COLLAPSED.height);
            void setBarHeight(ISLAND_COLLAPSED.height);
          }}
          onClick={() => {
            // 左滑划掉 / 明显滑动后忽略 click，避免误开应用
            if (swipe.current?.dismissed || swipe.current?.moved) return;
            const banner = msgBannerRef.current;
            if (banner) {
              // 插件通知：点横幅 → 下拉该插件面板看详情；左右按钮走 actions（非托盘跳转）
              if (banner.source === "plugin") {
                const pluginId = banner.pluginId;
                dismissMsgBanner();
                if (pluginId) {
                  const runtime = pluginRegistry.get(pluginId);
                  const hasPanel =
                    !!runtime?.enabled &&
                    !!runtime.manifest.slots?.["island.panel"] &&
                    (runtime.manifest.capabilities ?? []).includes("island.panel");
                  if (hasPanel) void openPluginSession(pluginId);
                }
                return;
              }
              // 托盘闪动通知：点横幅 → 唤起对应托盘应用
              void (async () => {
                try {
                  if (
                    banner.hwnd != null &&
                    banner.callbackMsg != null &&
                    banner.uid != null
                  ) {
                    await invoke("invoke_tray_icon", {
                      id: banner.trayIconId,
                      hwnd: banner.hwnd,
                      callbackMsg: banner.callbackMsg,
                      uid: banner.uid,
                      version: banner.version ?? 0,
                      action: "left",
                    });
                  }
                } catch (err) {
                  console.error(err);
                } finally {
                  // 点开后 flashing→0，否则会一直占着「已闪动」导致下次新消息不再提示
                  try {
                    await invoke("clear_tray_attention", {
                      id: banner.trayIconId,
                      hwnd: banner.hwnd ?? 0,
                      uid: banner.uid ?? 0,
                    });
                  } catch {
                    /* noop */
                  }
                  dismissMsgBanner();
                }
              })();
              return;
            }
            if (expandedRef.current && !busy.current) void collapse();
          }}
          onKeyDown={onIslandKeyDown}
        >
          <div
            ref={shapeLayerRef}
            className="island-shape-layer"
            style={{ width: size.width, height: size.height }}
            aria-hidden
          >
            <svg
              ref={svgRef}
              className="island-svg"
              viewBox={`0 0 ${size.width} ${size.height}`}
              width={size.width}
              height={size.height}
            >
              <path
                ref={pathRef}
                className="island-path"
                style={{ transition: "fill-opacity 240ms ease" } as CSSProperties}
              />
            </svg>
          </div>

          <div
            ref={islandUiRef}
            className="island-ui"
            style={{ width: size.width, minHeight: size.height }}
          >
            <div className={`island-bar${msgBanner ? " is-notifying" : ""}`}>
              <div className={`bar-weather${msgBanner ? " is-exiting" : ""}`}>
                {stagingBar ? (
                  <div
                    className="bar-staging"
                    role="button"
                    tabIndex={0}
                    title={islandBar?.title || "打开面板"}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      // 常驻摘要仅展示；展开一律用设置里的「下拉内容」
                      if (!expandedRef.current) void expand();
                    }}
                    onKeyDown={(e) => {
                      if (e.key !== "Enter" && e.key !== " ") return;
                      e.preventDefault();
                      e.stopPropagation();
                      if (!expandedRef.current) void expand();
                    }}
                  >
                    <span className="bar-staging-dot" aria-hidden />
                    <span className="bar-staging-text">{stagingBar}</span>
                  </div>
                ) : null}
              </div>
              {msgBanner ? (
                <div className="bar-notify" key={msgBanner.key} ref={notifyRef}>
                  <div className="bar-notify-slot is-start">
                    {(() => {
                      const act = actionsForSlot(msgBanner.actions, "start");
                      if (!act) return null;
                      return (
                        <button
                          type="button"
                          className="bar-notify-action"
                          style={{ background: act.background }}
                          title={act.label || act.id}
                          onPointerDown={(e) => e.stopPropagation()}
                          onClick={(e) => {
                            e.stopPropagation();
                            fireNotifyAction(act);
                          }}
                        >
                          {act.iconPng ? (
                            <img
                              className="bar-notify-action-icon"
                              src={`data:image/png;base64,${act.iconPng}`}
                              alt=""
                              draggable={false}
                            />
                          ) : (
                            act.label
                          )}
                        </button>
                      );
                    })()}
                  </div>
                  <div className="bar-notify-main">
                    {msgBanner.iconPng ? (
                      <img
                        className="bar-notify-icon"
                        src={`data:image/png;base64,${msgBanner.iconPng}`}
                        alt=""
                        draggable={false}
                      />
                    ) : (
                      <span className="bar-notify-fallback" aria-hidden>
                        {(msgBanner.title || "消").charAt(0).toUpperCase()}
                      </span>
                    )}
                    <span className="bar-notify-text">{msgBanner.text}</span>
                  </div>
                  <div className="bar-notify-slot is-end">
                    {(() => {
                      const act = actionsForSlot(msgBanner.actions, "end");
                      if (!act) return null;
                      return (
                        <button
                          type="button"
                          className="bar-notify-action"
                          style={{ background: act.background }}
                          title={act.label || act.id}
                          onPointerDown={(e) => e.stopPropagation()}
                          onClick={(e) => {
                            e.stopPropagation();
                            fireNotifyAction(act);
                          }}
                        >
                          {act.iconPng ? (
                            <img
                              className="bar-notify-action-icon"
                              src={`data:image/png;base64,${act.iconPng}`}
                              alt=""
                              draggable={false}
                            />
                          ) : (
                            act.label
                          )}
                        </button>
                      );
                    })()}
                  </div>
                </div>
              ) : null}
            </div>

            <div
              ref={panelRef}
              className={`island-panel is-plugin${pluginStagingShell ? " is-plugin-sized" : ""}`}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => e.stopPropagation()}
            >
              <IslandPanelHost
                pullContent={effectivePullContent}
                active={panelActive}
                onPanelClose={() => {
                  if (expandedRef.current) void collapse();
                }}
              />
            </div>
          </div>
        </div>
      </BorderBeam>
    </div>
  );
}

export default App;
