import { lerp, clamp01, pullProgress, easeOutSmooth, channelEase } from "./features/island/motion";
import { createIslandGeometry, ISLAND_CORNER_PATCH_SIZE } from "./features/island/geometry";
import { resolveIslandPullContent } from "./features/island/pullContent";
import { chromeTokens, chromeCssVars, type Rgb } from "./features/chrome/tokens";
import { sampleStripBands } from "./features/chrome/sampleStripBands";
import { AmbientStrip } from "./features/chrome/AmbientStrip";
import { useTrayNotificationFocus } from "./features/chrome/useTrayNotificationFocus";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type KeyboardEvent as ReactKeyboardEvent, type DragEvent as ReactDragEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import {
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
} from "@tauri-apps/api/window";
import { BorderBeam } from "border-beam";
import { dominantColorFromPngBase64 } from "./iconDominantColor";
import TrayCluster from "./components/TrayCluster";
import ChromeStatusCluster from "./components/ChromeStatusCluster";
import ShortcutsHost from "./components/ShortcutsHost";
import StatusMenu from "./components/StatusMenu";
import IslandPanelHost from "./components/IslandPanelHost";

/** Must match Rust `tray::TRAY_BOOT_ENABLED_DEFAULT` after slim rewrite. */
const TRAY_UI_ENABLED = true;

function setTrayUiPaused(paused: boolean) {
  if (!TRAY_UI_ENABLED) return;
  void invoke("set_tray_ui_paused", { paused }).catch(() => undefined);
}
import {
  applyIslandPrefsSnapshot,
  getIslandPrefs,
  hydrateIslandPrefs,
  refreshIslandPrefsFromDb,
  setIslandPrefs,
  subscribeIslandPrefs,
  clampStagingPanelH,
  clampStagingPanelW,
  STAGING_PANEL_H_DEFAULT,
  type IslandPrefs,
} from "./islandPrefs";
import {
  syncGlassCss,
  subscribeSystemDark,
  type GlassPrefs,
} from "./glassPrefs";
import { islandNotifyBus, type IslandNotifyBanner } from "./plugins/islandNotify";
import {
  actionsForSlot,
  normalizeNotifyActions,
  type NotifyAction,
} from "./plugins/notifyActions";
import { bootstrapPlugins, subscribeInstalledPlugins, arePluginsReady } from "./plugins/bootstrap";
import { normalizeStagingChanged } from "./stagingApi";
import {
  formatStagingBarText,
  isStagingPanelShell,
  islandSearchScenarioClaimOk,
  ISLAND_SEARCH_PLUGIN_ID,
  measureIslandBarLabelWidth,
  resolveIslandBarAdaptive,
  resolveIslandDropPluginId,
  resolveIslandFileSearchPluginId,
  resolveIslandSearchPluginId,
  resolvePluginPanelShellSize,
  type IslandBarState,
} from "./plugins/islandSlots";
import { parsePluginPanelId } from "./plugins/panelProviders";
import { pluginRegistry } from "./plugins/registry";
import {
  scenarioPresenceOk,
  trayKeyOf,
  trayKeysMatch,
  windowKeyOf,
} from "./scenarioGates";
import { hideChromeHoverTip, hostTipPointerProps, installChromeHoverTipGlobalDismiss } from "./chromeHoverTip";
import { clickTrace } from "./clickTrace";
import "./App.css";
import type { WindowInfo } from "./types";

/** 默认插件面板展开尺寸（非中转站） */
const VIEW_W_DEFAULT = 380;
const VIEW_H_DEFAULT = 220;
/** 岛贴屏顶后顶隙为 0；窗口高度 = 岛高（+ 冲突通知叠层） */
const TOP_GAP = 0;
const ISLAND_BAR_H = 28;
/** 冲突通知：主岛下方独立胶囊与顶边距 */
const NOTIFY_STACK_GAP = 4;
const NOTIFY_STACK_H = ISLAND_BAR_H;
const ISLAND_COLLAPSED_W_DEFAULT = 300;
/** Alt+Space 搜索态折叠岛宽（容纳搜索框） */
const ISLAND_SEARCH_COLLAPSED_W = 460;
/** 搜索框 ↔ 常驻摘要交接时长（与 .bar-weather 过渡对齐） */
const SEARCH_CHROME_EXIT_MS = 420;
/** 折叠目标宽（自适应歌词等）；与 liveExpanded 一样由 App 同步 */
const liveCollapsed = { width: ISLAND_COLLAPSED_W_DEFAULT, height: ISLAND_BAR_H };
function collapsedNow(): IslandSize {
  return { width: liveCollapsed.width, height: liveCollapsed.height };
}
/** 当前展开目标 / SVG 画布（中转站时变宽变矮）——由 App 每帧同步 */
const liveExpanded = { width: VIEW_W_DEFAULT, height: VIEW_H_DEFAULT };
/** 冲突通知叠层占用的窗口附加高度（与 paint 岛高解耦） */
let liveNotifyStackExtra = 0;
const HEIGHT_MS = 280;
/** 展开/收起总时长：宽高交错，禁止出现「380×28 宽扁直角条」中间态 */
const MORPH_MS = 420;
const PULL_OPEN = 0.52;
const CLICK_SLOP = 6;

const { islandBottomRadius, islandPath, islandNotifyInnerStrokePath, islandNotifyClipSilhouette } =
  createIslandGeometry(STAGING_PANEL_H_DEFAULT);

type IslandSize = { width: number; height: number };

function enabledPullContent(raw: string): string {
  const pid = parsePluginPanelId(raw);
  if (!pid) return raw || "";
  return pluginRegistry.get(pid)?.enabled ? raw : "";
}

/** 窗口实际高度 = 岛高 + 可选冲突通知叠层 */
function winHeight(islandH: number) {
  return TOP_GAP + islandH + liveNotifyStackExtra;
}


/** 右上：原点贴岛右上角，扇形在内侧，外轮廓为凹弧 */
const ISLAND_CORNER_PATCH_D_RIGHT = "M34 0C15.2223 0 0 15.2223 0 34V0H34Z";
/** 左上：水平镜像 */
const ISLAND_CORNER_PATCH_D_LEFT = "M0 0C18.7777 0 34 15.2223 34 34V0H0Z";
/** 顶边向上多画 1px，盖住 WebView/DPI 发丝缝 */
const ISLAND_TOP_BLEED = 1;

function IslandCornerPatches() {
  return (
    <>
      {/* 实色顶盖：盖住 SVG 顶边抗锯齿发丝缝（折叠岛贴屏时尤甚） */}
      <div className="island-top-cap" aria-hidden />
      <svg
        className="island-corner-patch island-corner-patch--left"
        width={ISLAND_CORNER_PATCH_SIZE}
        height={ISLAND_CORNER_PATCH_SIZE}
        viewBox="0 0 34 34"
        aria-hidden
      >
        <path className="island-corner-patch-fill" d={ISLAND_CORNER_PATCH_D_LEFT} />
      </svg>
      <svg
        className="island-corner-patch island-corner-patch--right"
        width={ISLAND_CORNER_PATCH_SIZE}
        height={ISLAND_CORNER_PATCH_SIZE}
        viewBox="0 0 34 34"
        aria-hidden
      >
        <path className="island-corner-patch-fill" d={ISLAND_CORNER_PATCH_D_RIGHT} />
      </svg>
    </>
  );
}

type Material = "none" | "mica-alt" | "blur" | "aero" | "acrylic";

type Ambient = {
  r: number;
  g: number;
  b: number;
  hwnd?: number;
  width?: number;
  offset_x?: number;
  span_width?: number;
  png_base64?: string;
};

type TrayAttention = {
  id: string;
  pin_key?: string;
  tooltip: string;
  process: string;
  icon_png_base64: string;
  hwnd: number;
  uid: number;
  callback_msg: number;
  version: number;
};

/** 暂时关掉 BorderBeam 炫彩外框，改用图标主色左边 1px */
const USE_NOTIFY_BORDER_BEAM = false;

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
  /** 托盘/通知图标面积最大色，用于左侧描边 */
  accentColor?: string;
  /** 调试：取色摘要（控制台 + 岛上色块） */
  accentDebug?: string;
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
/** Last Win32 height we applied — NEVER probe via innerSize/scaleFactor after expand
 * (click-trace #138→HUNG: shrink path deadlocked on those IPC queries). */
let lastAppliedBarWinH = ISLAND_BAR_H;
/** Serialize setBarHeight — morph used to overlap dozens of resize IPC calls. */
let barHeightTail: Promise<void> = Promise.resolve();
let barHeightSeq = 0;
/** 供 Win32 顶栏材质裁剪：展开时 = 顶栏条 ∪ 岛壳 */
let liveIslandClip = { width: ISLAND_COLLAPSED_W_DEFAULT, height: ISLAND_BAR_H };

async function setBarHeight(islandH: number) {
  // Integer px only — fractional morph steps must not each SetWindowPos.
  const h = Math.round(islandH);
  const seq = ++barHeightSeq;
  const run = async () => {
    // Latest wins: drop superseded morph-frame requests.
    if (seq !== barHeightSeq) {
      clickTrace("fe-island", `setBarHeight skip superseded h=${h}`);
      return;
    }
    await setBarHeightInner(h);
  };
  barHeightTail = barHeightTail.then(run, run);
  return barHeightTail;
}

async function setBarHeightInner(islandH: number) {
  clickTrace("fe-island", `setBarHeight enter h=${islandH}`);
  if (cachedScreenW == null) {
    clickTrace("fe-island", "before screenLogicalWidth");
    cachedScreenW = await screenLogicalWidth();
    clickTrace("fe-island", `after screenLogicalWidth w=${cachedScreenW}`);
  }
  const width = cachedScreenW;
  const targetH = winHeight(islandH);
  const raised = islandH > ISLAND_BAR_H + 2;
  if (raised) {
    try {
      clickTrace("fe-island", "before float_overlay");
      await invoke("float_overlay");
      clickTrace("fe-island", "after float_overlay");
    } catch {
      /* noop outside tauri */
    }
  }
  // Compare against last applied height only — querying HWND via Tauri
  // innerSize/scaleFactor after expand freezes WebView2 (proven HUNG dump).
  // Use 4px slack so morph float noise never retriggers resize.
  const needSize = Math.abs(lastAppliedBarWinH - targetH) > 4;
  clickTrace(
    "fe-island",
    `needSize=${needSize} last=${lastAppliedBarWinH} target=${targetH}`,
  );
  if (needSize) {
    try {
      if (!raised) {
        // BEFORE setSize — same suppress as resize_main_island. Without it,
        // Moved → bar_comp @200ms races tray-icons / WebView paint → HUNG
        // (click-trace: after setSize collapse → leave → HUNG ~3s).
        try {
          await invoke("suppress_island_bar_refresh", { ms: 800 });
        } catch {
          /* noop */
        }
        clickTrace("fe-island", `before setSize collapse h=${targetH}`);
        await getCurrentWindow().setSize(new LogicalSize(width, targetH));
        lastAppliedBarWinH = targetH;
        clickTrace("fe-island", "after setSize collapse");
      } else {
        clickTrace("fe-island", `before resize_main_island h=${targetH}`);
        await invoke("resize_main_island", { windowHeight: targetH });
        lastAppliedBarWinH = targetH;
        clickTrace("fe-island", "after resize_main_island");
      }
    } catch (e) {
      clickTrace("fe-island", `resize error ${String(e)}`);
      try {
        await getCurrentWindow().setSize(new LogicalSize(width, targetH));
        lastAppliedBarWinH = targetH;
      } catch {
        /* noop */
      }
    }
  }
  try {
    const raisedIsland = islandH > ISLAND_BAR_H + 2;
    const clipW = raisedIsland
      ? Math.max(liveIslandClip.width, liveExpanded.width)
      : liveIslandClip.width;
    const clipH = raisedIsland
      ? Math.max(islandH, liveIslandClip.height, liveExpanded.height)
      : islandH;
    liveIslandClip = { width: clipW, height: clipH };
    // Expand only: fire-and-forget material when HWND moved.
    // Collapse/boot: ZERO settle/reassert IPC — settle SetWindowPos + bar_comp
    // after resize hung the pump (click-trace #85 / boot #29 HUNG).
    if (raised && needSize) {
      clickTrace(
        "fe-island",
        `reassert fire-forget w=${clipW} h=${clipH}`,
      );
      void invoke("reassert_main_bar_geometry", {
        islandWidth: clipW,
        islandHeight: clipH,
      }).catch(() => undefined);
    } else if (!raised) {
      // Clear TOPMOST after collapse — previously skipped to avoid HUNG; settle is
      // now flag+clear_topmost only (no ShowWindow). Without this, expand left
      // OVERLAY_RAISED stuck and watchdog kept re-TOPMOST-ing the strip.
      clickTrace(
        "fe-island",
        needSize
          ? "shrink: settle_overlay (clear topmost)"
          : "bar-height no-op: settle_overlay",
      );
      void invoke("settle_overlay").catch(() => undefined);
    }
  } catch {
    /* noop */
  }
  clickTrace("fe-island", `setBarHeight leave h=${islandH}`);
}

/** Sync CSS theme + let Rust decide Win32 glass (desktop always on). */
async function applyBarMaterial(): Promise<{ dark?: boolean; kind?: Material }> {
  try {
    const prefs = await invoke<GlassPrefs>("get_material_prefs");
    const dark = await syncGlassCss(prefs);
    const kind = await invoke<string>("apply_window_effect", {});
    return { dark, kind: (kind as Material) || "mica-alt" };
  } catch {
    return {};
  }
}

function App() {
  const [expanded, setExpanded] = useState(false);
  const [trayOpen, setTrayOpen] = useState(false);
  const [statusMenuOpen, setStatusMenuOpen] = useState(false);
  /** Hidden until Rust host-boot-ready (tray seeded + chrome reveal). */
  const [bootReady, setBootReady] = useState(false);
  const [material, setMaterial] = useState<Material>("mica-alt");
  const [ambient, setAmbient] = useState<Ambient>({ r: 32, g: 32, b: 34, hwnd: 0 });
  const [barGlassDark, setBarGlassDark] = useState(true);
  const [chromeLeft, setChromeLeft] = useState(() => chromeTokens({ r: 32, g: 32, b: 34 }));
  const [chromeCenter, setChromeCenter] = useState(() => chromeTokens({ r: 32, g: 32, b: 34 }));
  const [chromeRight, setChromeRight] = useState(() => chromeTokens({ r: 32, g: 32, b: 34 }));
  const [size, setSize] = useState<IslandSize>(collapsedNow());
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
  /** 常驻层：仅全局设置选中的 barResident 插件可写 */
  const [residentBar, setResidentBar] = useState<IslandBarState | null>(null);
  /** 临时层：中转站等；有内容时盖住常驻与情景 */
  const [overlayBar, setOverlayBar] = useState<IslandBarState | null>(null);
  /** 情景临时：claimScenario 后写入；释放后回到常驻 */
  const [scenarioOwner, setScenarioOwner] = useState<string | null>(null);
  const [scenarioBar, setScenarioBar] = useState<IslandBarState | null>(null);
  const [scenarioPull, setScenarioPull] = useState<string | null>(null);
  const scenarioPullRef = useRef(scenarioPull);
  const residentBarRef = useRef(residentBar);
  const overlayBarRef = useRef(overlayBar);
  const scenarioOwnerRef = useRef(scenarioOwner);
  const scenarioBarRef = useRef(scenarioBar);
  const liveTrayKeysRef = useRef<string[]>([]);
  const liveWindowKeysRef = useRef<string[]>([]);
  const barStagingTextRef = useRef<HTMLSpanElement>(null);
  const collapsedSizeTimer = useRef<number | null>(null);
  const syncCollapsedIslandWidthRef = useRef<(nextW: number) => void>(() => undefined);
  const widthForBarLabelRef = useRef<
    (text: string, pluginId: string | null | undefined, showDot: boolean, isDrop: boolean) => number
  >(() => ISLAND_COLLAPSED_W_DEFAULT);
  residentBarRef.current = residentBar;
  overlayBarRef.current = overlayBar;
  scenarioOwnerRef.current = scenarioOwner;
  scenarioPullRef.current = scenarioPull;
  scenarioBarRef.current = scenarioBar;

  function clearScenarioLayer() {
    scenarioOwnerRef.current = null;
    setScenarioOwner(null);
    setScenarioBar(null);
    setScenarioPull(null);
  }

  function scenarioGateAllows(pluginId: string): boolean {
    return scenarioPresenceOk(
      pluginId,
      islandPrefsRef.current.scenarioGates,
      liveTrayKeysRef.current,
      liveWindowKeysRef.current,
    );
  }

  function refreshPresenceKeys(
    trays?: Array<{ id: string; pin_key?: string }>,
    windows?: WindowInfo[],
  ) {
    if (trays) {
      liveTrayKeysRef.current = trays
        .map((t) => trayKeyOf(t))
        .filter(Boolean);
    }
    if (windows) {
      liveWindowKeysRef.current = windows
        .map((w) => windowKeyOf(w))
        .filter(Boolean);
    }
    const owner = scenarioOwnerRef.current;
    if (owner && !scenarioGateAllows(owner)) {
      clearScenarioLayer();
    }
  }

  /** 折叠岛宽：即时 paintDom 居中变宽；debounce 的是 React size（ShortcutsHost 用） */
  function syncCollapsedIslandWidth(nextW: number) {
    const w = Math.max(28, Math.round(nextW));
    liveCollapsed.width = w;
    if (!expandedRef.current && revealRef.current < 0.02 && !pullingRef.current) {
      const root = islandRef.current;
      if (root) {
        // 显式钉住水平居中，避免宽度动画/重绘时漂向一侧
        root.style.left = "50%";
        root.style.right = "auto";
        root.style.setProperty("translate", "-50% 0");
      }
      paintDom({ width: w, height: ISLAND_BAR_H }, revealRef.current);
    }
    if (collapsedSizeTimer.current != null) {
      window.clearTimeout(collapsedSizeTimer.current);
    }
    collapsedSizeTimer.current = window.setTimeout(() => {
      collapsedSizeTimer.current = null;
      setSize((prev) => {
        if (Math.abs(prev.width - w) < 1 && prev.height === ISLAND_BAR_H) return prev;
        return { width: w, height: ISLAND_BAR_H };
      });
    }, 64);
  }

  function widthForBarLabel(
    text: string,
    pluginId: string | null | undefined,
    showDot: boolean,
    isDrop: boolean,
  ) {
    const trimmed = text.trim();
    if (!trimmed) return ISLAND_COLLAPSED_W_DEFAULT;
    const adaptive = resolveIslandBarAdaptive(pluginId);
    if (!(adaptive.enabled || isDrop)) return ISLAND_COLLAPSED_W_DEFAULT;
    const measured = measureIslandBarLabelWidth(trimmed, { showDot });
    const minW = isDrop ? 220 : adaptive.minWidth;
    const maxW = isDrop ? 420 : adaptive.maxWidth;
    return Math.min(maxW, Math.max(minW, measured));
  }

  syncCollapsedIslandWidthRef.current = syncCollapsedIslandWidth;
  widthForBarLabelRef.current = widthForBarLabel;
  const [dropTarget, setDropTarget] = useState(false);
  const [panelOverride, setPanelOverride] = useState<string | null>(null);
  const panelSessionRef = useRef<string | null>(null);
  const panelSessionArmedRef = useRef(false);

  function clearSessionPanel() {
    panelSessionRef.current = null;
    panelSessionArmedRef.current = false;
    setPanelOverride(null);
  }

  /** 会话 override 仅应在展开态有效；收起后残留会盖住「下拉内容」设置 */
  function clearStalePanelOverride() {
    if (!expandedRef.current && revealRef.current <= 0.01) {
      clearSessionPanel();
    }
  }
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
  /** Alt+Space 全局搜索：岛栏变搜索框 + 打开 everything 面板会话 */
  const [searchMode, setSearchMode] = useState(false);
  const [searchDraft, setSearchDraft] = useState("");
  const [searchSubmit, setSearchSubmit] = useState<{
    nonce: number;
    query: string;
    action?: string;
  } | null>(null);
  const searchModeRef = useRef(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  /** collapse 时保留搜索态（Alt+空格从展开切回「仅搜索栏」） */
  const retainSearchModeRef = useRef(false);
  const toggleIslandSearchHotkeyRef = useRef<(() => void | Promise<void>) | null>(
    null,
  );
  const openFavoritesHotkeyRef = useRef<(() => void | Promise<void>) | null>(
    null,
  );
  const handoffFileSearchRef = useRef<(query: string) => void>(() => undefined);
  /** Esc 退出过渡：先播动画再卸 DOM */
  const [searchLeaving, setSearchLeaving] = useState(false);
  const searchLeavingRef = useRef(false);
  const searchLeaveTimerRef = useRef<number | null>(null);
  /** 热键连按防抖：enter 异步未完成时忽略重复 toggle */
  const searchToggleBusyRef = useRef(false);
  /**
   * 搜索 chrome 只认 Host searchMode / 离场动画，不跟 scenarioOwner 抖动
   *（正在播放等会抢 claim，不能让输入框跟着丢）。
   */
  const showSearchChrome = searchMode || searchLeaving;
  const searchActive = searchMode;
  /** 搜索锁期间 Host chrome 优先于中转站 overlay（情景临时 > 常驻；搜索为 Host 情景） */
  const islandBar = showSearchChrome
    ? scenarioBar ?? residentBar
    : overlayBar ?? scenarioBar ?? residentBar;
  /** 托盘闪动消息提示（岛内落下） */
  const [msgBanner, setMsgBanner] = useState<MsgBanner | null>(null);
  const gen = useRef(0);
  const busy = useRef(false);
  const expandedRef = useRef(expanded);
  const trayOpenRef = useRef(trayOpen);
  const sizeRef = useRef(size);
  const revealRef = useRef(0);
  const immersedRef = useRef(false);
  const islandPrefsRef = useRef(islandPrefs);
  /** pin_key / id → flash→island; missing = true. Synced from tray-prefs. */
  const trayFlashNotifyRef = useRef<Record<string, boolean>>({});
  /** Tray ids we already surfaced on the island for the current flash episode. */
  const trayBannerShownRef = useRef<Set<string>>(new Set());
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
  const pathClipRef = useRef<SVGPathElement>(null);
  const pathStrokeRef = useRef<SVGPathElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const shapeLayerRef = useRef<HTMLDivElement>(null);
  const islandUiRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const lastWinH = useRef(winHeight(ISLAND_BAR_H));
  /** 窗口真实逻辑高度（lastWinH 可能被预拉高污染，拖拽必须以实测为准） */
  const actualWinHRef = useRef(winHeight(ISLAND_BAR_H));
  /** 悬停离开后的窗口收回 debounce，避免「预拉高 → 离开收回 → 按下拖拽」竞态裁切 */
  const leaveShrinkTimer = useRef<number | null>(null);
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
  // Do NOT write shellPanelW/H refs from React state here — lagged 380×220 state
  // was clobbering a correct 400×200 sync mid-expand (two island sizes).
  // size / reveal 只由 paintDom 维护，避免重渲染把动画进度打回旧值

  /** 面板尺寸：refs 优先，避免 expand/morph 中 React state 滞后把 liveExpanded 打回 380×220 */
  function resolvePanelSizingPluginId(): string | null {
    const owner = scenarioOwnerRef.current;
    if (owner) {
      const sp = scenarioPullRef.current ?? `plugin:${owner}`;
      const v = enabledPullContent(sp);
      if (v) return parsePluginPanelId(v);
    }
    if (panelSessionArmedRef.current && panelSessionRef.current) {
      const v = enabledPullContent(panelSessionRef.current);
      if (v) return parsePluginPanelId(v);
    }
    return parsePluginPanelId(
      enabledPullContent(islandPrefsRef.current.pullContent),
    );
  }

  function isFileSearchPlugin(pluginId: string | null | undefined): boolean {
    if (!pluginId) return false;
    const base = pluginId.replace(/__dev$/, "");
    return (
      base === ISLAND_SEARCH_PLUGIN_ID ||
      pluginId === resolveIslandSearchPluginId()
    );
  }

  /** Alt+空格 Host 搜索锁：期间禁止其它情景 claim / setBar 抢主人 */
  function hostSearchLocksScenario(): boolean {
    return searchModeRef.current || searchLeavingRef.current;
  }

  /** 按当前岛栏文案重算折叠尺寸（收起结束时用，避免 liveCollapsed 过期导致错位） */
  function snapCollapsedFromBar(): IslandSize {
    if (searchModeRef.current) {
      liveCollapsed.width = ISLAND_SEARCH_COLLAPSED_W;
      liveCollapsed.height = ISLAND_BAR_H;
      return { width: ISLAND_SEARCH_COLLAPSED_W, height: ISLAND_BAR_H };
    }
    const overlay = overlayBarRef.current;
    const scenario = scenarioBarRef.current;
    const resident = residentBarRef.current;
    const text = String(overlay?.text ?? scenario?.text ?? resident?.text ?? "");
    const pluginId =
      overlay?.pluginId ?? scenario?.pluginId ?? resident?.pluginId ?? null;
    const showDot = Boolean(
      pluginId &&
        pluginRegistry.get(pluginId)?.manifest.slots?.["island.bar"]
          ?.excludeFromBarResident,
    );
    const w = text.trim()
      ? widthForBarLabel(text, pluginId, showDot, false)
      : ISLAND_COLLAPSED_W_DEFAULT;
    liveCollapsed.width = w;
    liveCollapsed.height = ISLAND_BAR_H;
    return { width: w, height: ISLAND_BAR_H };
  }

  useEffect(() => installChromeHoverTipGlobalDismiss(), []);

  // 情景主人已是文件搜索 → 强制亮搜索 chrome（仅作兜底；主路径以 searchMode 为准）
  useEffect(() => {
    if (!isFileSearchPlugin(scenarioOwner)) return;
    if (searchModeRef.current || searchLeavingRef.current) return;
    searchModeRef.current = true;
    setSearchMode(true);
    liveCollapsed.width = ISLAND_SEARCH_COLLAPSED_W;
    if (!expandedRef.current) {
      syncCollapsedIslandWidth(ISLAND_SEARCH_COLLAPSED_W);
    }
  }, [scenarioOwner]);

  // Alt+空格进入搜索态后，强制主窗焦点 + 输入框聚焦（与 claim 异步解耦）
  useLayoutEffect(() => {
    if (!searchActive || searchLeaving || expanded || pulling || springing) return;
    void focusIslandSearchInput();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchActive, searchLeaving, expanded, pulling, springing]);

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
      setResidentBar((prev) => {
        if (!prev) return prev;
        const want = islandPrefsRef.current.barResident;
        if (!want || prev.pluginId !== want || !barOk(prev.pluginId)) return null;
        return prev;
      });
      setOverlayBar((prev) => {
        if (!prev) return prev;
        return barOk(prev.pluginId) ? prev : null;
      });
      setScenarioOwner((owner) => {
        if (!owner) return owner;
        const rec = pluginRegistry.get(owner);
        const ok =
          Boolean(rec?.enabled) &&
          Boolean(rec?.manifest.slots?.["island.scenario"]) &&
          barOk(owner);
        if (ok) return owner;
        // 文件搜索被禁用/卸载：完整退出搜索会话（含展开态与窗口高度）
        if (isFileSearchPlugin(owner)) {
          const searchLive =
            searchModeRef.current ||
            searchLeavingRef.current ||
            expandedRef.current ||
            panelSessionRef.current;
          if (searchLive) {
            exitIslandSearchChrome({ animated: false });
            if (expandedRef.current) void collapse();
            void shrinkIslandWindow();
          }
        }
        setScenarioBar(null);
        setScenarioPull(null);
        return null;
      });
      setScenarioBar((prev) => {
        if (!prev) return prev;
        return barOk(prev.pluginId) &&
          scenarioOwnerRef.current === prev.pluginId
          ? prev
          : null;
      });
      setScenarioPull((prev) => {
        if (!prev?.startsWith("plugin:")) return prev;
        const pid = prev.slice("plugin:".length);
        return pluginRegistry.get(pid)?.enabled &&
          scenarioOwnerRef.current === pid
          ? prev
          : null;
      });
      setPanelOverride((prev) => {
        if (!prev?.startsWith("plugin:")) return prev;
        const pid = prev.slice("plugin:".length);
        if (pluginRegistry.get(pid)?.enabled) return prev;
        panelSessionRef.current = null;
        panelSessionArmedRef.current = false;
        return null;
      });
      // Drop cached pull / bar targets only after plugins are loaded.
      // Before bootstrap, registry is empty — treating that as "disabled" wiped prefs on every restart.
      if (arePluginsReady()) {
        const pull = islandPrefsRef.current.pullContent;
        if (pull.startsWith("plugin:")) {
          const pid = pull.slice("plugin:".length);
          // Only clear if uninstalled — disabled plugins keep preference for re-enable.
          if (pid && !pluginRegistry.get(pid)) {
            void setIslandPrefs({ pullContent: "" }).catch(() => undefined);
          }
        }
        const wantBar = islandPrefsRef.current.barResident;
        if (wantBar && !pluginRegistry.get(wantBar)) {
          void setIslandPrefs({ barResident: "" }).catch(() => undefined);
        } else if (
          wantBar &&
          pluginRegistry.get(wantBar)?.manifest.slots?.["island.scenario"]
        ) {
          // Scenario plugins are not permanent 常驻 — drop stale selection.
          void setIslandPrefs({ barResident: "" }).catch(() => undefined);
        }
        if (pull.startsWith("plugin:")) {
          const pid = pull.slice("plugin:".length);
          if (
            pid &&
            pluginRegistry.get(pid)?.manifest.slots?.["island.scenario"]
          ) {
            void setIslandPrefs({ pullContent: "" }).catch(() => undefined);
          }
        }
      }
    };
    sync();
    return pluginRegistry.subscribe(sync);
  }, []);

  useEffect(() => {
    const want = islandPrefs.barResident;
    setResidentBar((prev) => {
      if (!want) return null;
      if (prev && prev.pluginId !== want) return null;
      return prev;
    });
    // 竞选常驻的插件不得留在临时层（修「无」之后改设置又刷出天气）
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

  function pluginEnabled(id: string | null | undefined): boolean {
    if (!id) return false;
    return Boolean(pluginRegistry.get(id)?.enabled);
  }

  /** 用户配置了可用的下拉插件面板 */
  function hasConfiguredPullContent(): boolean {
    const prefId = parsePluginPanelId(islandPrefsRef.current.pullContent);
    if (!prefId) return false;
    return pluginEnabled(prefId);
  }

  function barResidentPanelPluginId(): string | null {
    const pref = islandPrefsRef.current.barResident?.trim();
    if (!pref || !pluginEnabled(pref)) return null;
    const rec = pluginRegistry.get(pref);
    if (!rec) return null;
    if (!rec.manifest.slots?.["island.panel"]) return null;
    if (!(rec.manifest.capabilities ?? []).includes("island.panel")) return null;
    return pref;
  }

  /** 壳层点击/下拉应打开的插件：情景 claim 优先（与实际渲染一致），再 prefs 下拉 / 岛栏常驻 */
  function resolveShellExpandPluginId(): string | null {
    const owner = scenarioOwnerRef.current;
    if (owner) {
      const sp = scenarioPullRef.current ?? `plugin:${owner}`;
      const fromScenario = parsePluginPanelId(enabledPullContent(sp));
      if (fromScenario) return fromScenario;
    }
    const pullPid = parsePluginPanelId(
      enabledPullContent(islandPrefsRef.current.pullContent),
    );
    if (pullPid) return pullPid;
    return barResidentPanelPluginId();
  }

  function syncLiveExpandedForPlugin(pluginId: string) {
    const { w, h } = resolvePluginPanelShellSize(pluginId, null, {
      w: clampStagingPanelW,
      h: clampStagingPanelH,
    });
    liveExpanded.width = w;
    liveExpanded.height = h;
    shellPanelWRef.current = w;
    shellPanelHRef.current = h;
    setShellPanelW(w);
    setShellPanelH(h);
  }

  /** Prefer local last-applied height — never query HWND (innerSize deadlocks after expand). */
  function readActualWinH(): number {
    return actualWinHRef.current;
  }

  async function raiseIslandWindow(islandH: number): Promise<void> {
    await setBarHeight(islandH);
    const targetWinH = winHeight(islandH);
    lastWinH.current = targetWinH;
    actualWinHRef.current = targetWinH;
    // No innerSize poll — each probe was a WebView2 IPC that can freeze the pump
    // (click-trace: expand OK → shrink enter h=28 → HUNG before resize).
  }

  async function shrinkIslandWindow(): Promise<void> {
    clickTrace("fe-island", "shrinkIslandWindow");
    await setBarHeight(ISLAND_BAR_H);
    const targetWinH = winHeight(ISLAND_BAR_H);
    lastWinH.current = targetWinH;
    actualWinHRef.current = targetWinH;
  }

  /** 用户配置了可用的岛栏常驻 */
  function hasConfiguredBarResident(): boolean {
    const pref = islandPrefsRef.current.barResident?.trim();
    if (!pref) return false;
    return pluginEnabled(pref);
  }

  /** 常驻+下拉皆「无」，且当前无摘要/通知 → 空黑岛，沉浸不等待 */
  function shouldImmerseWithoutDelay(): boolean {
    if (hasConfiguredPullContent() || hasConfiguredBarResident()) return false;
    if (msgBannerRef.current) return false;
    if (residentBarRef.current?.text?.trim()) return false;
    if (overlayBarRef.current?.text?.trim()) return false;
    return true;
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
    const banner = msgBannerRef.current;
    const id = banner?.notifyId;
    msgBannerRef.current = null;
    setMsgBanner(null);
    if (id) islandNotifyBus.dismiss(id);
    else islandNotifyBus.dismiss();
    // 划掉/关闭也要复位 flashing，否则托盘一直闪却不再发 tray-attention 上升沿
    if (banner?.source === "tray") {
      void invoke("clear_tray_attention", {
        id: banner.trayIconId,
        hwnd: banner.hwnd ?? 0,
        uid: banner.uid ?? 0,
      }).catch(() => undefined);
    }
    scheduleImmerse();
  }

  useTrayNotificationFocus(
    msgBanner?.source === "tray" ? msgBanner.notifyId : undefined,
    msgBanner?.hwnd,
    dismissMsgBanner,
  );

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

  function applyMsgBannerFromBus(b: IslandNotifyBanner) {
    // 「闪动时通知上岛」关闭时：绝不把托盘 attention 落到岛上（含收起后补弹 / bus 订阅）
    if (b.source === "tray" && !trayFlashNotifyAllowed({
      id: b.tray?.iconId,
      pinKey: b.tray?.pinKey,
    })) {
      console.info("[tray-attention] bus apply blocked: flash notify off", b.id);
      islandNotifyBus.dismiss(b.id);
      return;
    }
    bumpIslandActivity();
    clearIdleTimer();
    const next = bannerFromBus(b);
    msgBannerRef.current = next;
    setMsgBanner(next);
    const key = next.key;
    const png = next.iconPng;
    void dominantColorFromPngBase64(png).then((dbg) => {
      console.info("[notify-accent]", {
        key,
        source: next.source,
        title: next.title,
        ...dbg,
      });
      if (msgBannerRef.current?.key !== key) return;
      const patched = {
        ...msgBannerRef.current,
        accentColor: dbg.color,
        accentDebug: `${dbg.reason} · ${dbg.color} · top=${dbg.top.map((t) => `${t.color}×${t.n}`).join(" | ") || "∅"}`,
      };
      msgBannerRef.current = patched;
      setMsgBanner(patched);
    });
  }

  /** Global msgNotify + per-icon flash_notify (missing = on). */
  function trayFlashNotifyAllowed(keys: { id?: string; pinKey?: string }): boolean {
    if (!islandPrefsRef.current.msgNotify) return false;
    const map = trayFlashNotifyRef.current;
    const pin = (keys.pinKey || "").trim();
    const id = (keys.id || "").trim();
    if (pin && map[pin] === false) return false;
    if (id && map[id] === false) return false;
    for (const [k, v] of Object.entries(map)) {
      if (v !== false) continue;
      if (pin && trayKeysMatch(k, pin, liveTrayKeysRef.current)) return false;
      if (id && trayKeysMatch(k, id, liveTrayKeysRef.current)) return false;
    }
    return true;
  }

  /** 托盘闪动 → 通知总线（常驻，ttl=0） */
  function showMsgBanner(att: TrayAttention) {
    const prefs = islandPrefsRef.current;
    const pinKey = (att.pin_key || "").trim();
    if (!trayFlashNotifyAllowed({ id: att.id, pinKey })) {
      console.info("[tray-attention] skipped: flash notify off", att.id, pinKey);
      return;
    }
    if (trayBannerShownRef.current.has(att.id)) {
      return;
    }
    if (expandedRef.current || revealRef.current > 0.05) {
      console.info("[tray-attention] deferred: island busy", {
        id: att.id,
        expanded: expandedRef.current,
        reveal: revealRef.current,
      });
      return;
    }

    bumpIslandActivity();
    clearIdleTimer();

    const text = (prefs.msgNotifyText || "收到一条消息").trim() || "收到一条消息";
    const title = (att.tooltip || att.process || "").trim();
    trayBannerShownRef.current.add(att.id);

    const push = (iconPng: string) => {
      console.info("[tray-attention] show", {
        id: att.id,
        pinKey,
        title,
        iconBytes: (iconPng || "").length,
      });
      islandNotifyBus.push({
        source: "tray",
        title: title || text,
        body: text,
        iconPng: iconPng || undefined,
        urgency: "active",
        ttlMs: 0,
        tray: {
          iconId: att.id,
          pinKey: pinKey || undefined,
          hwnd: att.hwnd,
          uid: att.uid,
          callbackMsg: att.callback_msg,
          version: att.version ?? 0,
        },
      });
    };

    const existing = (att.icon_png_base64 || "").trim();
    if (existing) {
      push(existing);
      return;
    }
    // tray-icons 是 meta-only：补拉一次该 id 的托盘 PNG，避免灵动岛只显示「微」。
    void invoke<Record<string, string>>("get_tray_icon_glyphs", { ids: [att.id] })
      .then((map) => push(map?.[att.id] || ""))
      .catch(() => push(""));
  }

  /**
   * 补弹：HMR / 划掉未清 flashing / 错过上升沿时，tray-icons 里仍 flashing 则再推一次。
   * 也会把 bus 已有、UI 未挂上的横幅补上。
   */
  function syncFlashingTrayBanner(
    icons: Array<{
      id: string;
      pin_key?: string;
      tooltip: string;
      process: string;
      icon_png_base64: string;
      hwnd: number;
      uid: number;
      callback_msg: number;
      version?: number;
      flashing?: boolean;
      system_tray?: boolean;
    }>,
  ) {
    if (expandedRef.current || revealRef.current > 0.05) return;
    if (!islandPrefsRef.current.msgNotify) return;

    const pending = islandNotifyBus.getCurrent();
    if (pending && !msgBannerRef.current) {
      applyMsgBannerFromBus(pending);
      return;
    }
    if (msgBannerRef.current) return;

    const flashingIds = new Set(
      icons.filter((i) => i.flashing && !i.system_tray).map((i) => i.id),
    );
    for (const id of trayBannerShownRef.current) {
      if (!flashingIds.has(id)) trayBannerShownRef.current.delete(id);
    }

    const flashing = icons.find((i) => {
      if (!i.flashing || i.system_tray) return false;
      if (trayBannerShownRef.current.has(i.id)) return false;
      return trayFlashNotifyAllowed({
        id: i.id,
        pinKey: (i.pin_key || "").trim(),
      });
    });
    if (!flashing) return;
    showMsgBanner({
      id: flashing.id,
      pin_key: flashing.pin_key,
      tooltip: flashing.tooltip,
      process: flashing.process,
      icon_png_base64: flashing.icon_png_base64,
      hwnd: flashing.hwnd,
      uid: flashing.uid,
      callback_msg: flashing.callback_msg,
      version: flashing.version ?? 0,
    });
  }

  function scheduleImmerse() {
    clearIdleTimer();
    const prefs = islandPrefsRef.current;
    if (!prefs.autoImmerse) return;
    if (expandedRef.current) return;
    if (revealRef.current > 0.02) return;
    if (busy.current) return;
    if (msgBannerRef.current) return;
    // 常驻+下拉皆无且岛上空：立刻沉浸，勿留黑色空岛等 idle
    const delayMs = shouldImmerseWithoutDelay() ? 0 : prefs.immerseIdleSec * 1000;
    idleTimer.current = window.setTimeout(() => {
      idleTimer.current = null;
      const latest = islandPrefsRef.current;
      if (!latest.autoImmerse) return;
      if (expandedRef.current || busy.current) return;
      if (revealRef.current > 0.02) return;
      if (msgBannerRef.current) return;
      immersedRef.current = true;
      setImmersed(true);
    }, delayMs);
  }

  /** 壳层手势：下拉内容或岛栏常驻面板均可展开 */
  function canShellPullExpand(): boolean {
    return resolveShellExpandPluginId() != null;
  }

  /** 直接改 DOM；动画中不走 React，避免 ambient 等重渲染把路径打回旧值 */
  function paintDom(next: IslandSize, nextReveal: number) {
    sizeRef.current = next;
    revealRef.current = nextReveal;
    liveIslandClip = { width: next.width, height: next.height };
    // 顶角直角贴屏；壳层向上 bleed 1px，盖住 WebView 顶边发丝缝
    const topSquare = 1;
    const gap = 0;
    const bleed = ISLAND_TOP_BLEED;
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
    const patch = ISLAND_CORNER_PATCH_SIZE;
    // 壳层几何由 CSS（--island-corner-patch / bleed）承担，避免 React style 抹掉 left 导致错位
    if (shape) {
      shape.style.left = "";
      shape.style.top = "";
      shape.style.width = "";
      shape.style.height = "";
    }
    const svg = svgRef.current;
    if (svg) {
      svg.setAttribute(
        "viewBox",
        `${-patch} ${-bleed} ${w + patch * 2} ${h + bleed}`,
      );
      svg.removeAttribute("width");
      svg.removeAttribute("height");
    }
    const path = pathRef.current;
    const d = islandPath(w, h, topSquare, bleed);
    if (path) path.setAttribute("d", d);
    // clip = 岛身+补丁外轮廓，描边沿补丁凹弧（非直角贴屏）
    pathClipRef.current?.setAttribute(
      "d",
      islandNotifyClipSilhouette(w, h, patch, bleed),
    );
    pathStrokeRef.current?.setAttribute(
      "d",
      islandNotifyInnerStrokePath(w, h, patch),
    );
    const ui = islandUiRef.current;
    if (ui) {
      ui.style.width = "";
      ui.style.minHeight = "";
    }
    // panel is-open / opacity：React className + CSS（勿在此写 opacity，会闪黑）
    // NEVER raiseWindowIfNeeded here — morph rAF called this every frame with
    // lerp heights → SetWindowPos storm → HUNG on 收起 (lyrics dropdown).
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
        const expandW = shellPanelWRef.current;
        const expandH = shellPanelHRef.current;
        paintDom(
          {
            width: lerp(liveCollapsed.width, expandW, wE),
            height: lerp(liveCollapsed.height, expandH, hE),
          },
          rE,
        );
        if (p < 1) {
          requestAnimationFrame(step);
        } else {
          const end = opening
            ? { width: expandW, height: expandH }
            : collapsedNow();
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

  /** 点击展开：宽高交错长大，不经过宽扁中间态。force = 岛栏/拖入/通知打开会话。 */
  async function expand(opts?: { force?: boolean }) {
    clickTrace(
      "fe-island",
      `expand enter force=${!!opts?.force} busy=${busy.current} expanded=${expandedRef.current}`,
    );
    if (busy.current || expandedRef.current) return;
    if (!opts?.force) {
      clearSessionPanel();
      await refreshIslandPrefsFromDb().then(setIslandPrefsState);
      const pid = resolveShellExpandPluginId();
      if (!pid) return;
      armPluginSession(pid);
    } else {
      const pid =
        parsePluginPanelId(panelSessionRef.current ?? "") ??
        resolvePanelSizingPluginId();
      if (pid) syncLiveExpandedForPlugin(pid);
    }
    bumpIslandActivity();
    const token = ++gen.current;
    busy.current = true;
    setTrayUiPaused(true);
    trayOpenRef.current = false;
    setTrayOpen(false);
    pullingRef.current = false;
    setPulling(false);
    setSpringing(false);
    try {
      // 点击展开：立刻直角贴顶
      morphingRef.current = true;
      paintDom(sizeRef.current, revealRef.current);
      await raiseIslandWindow(shellPanelHRef.current);
      if (token !== gen.current) return;
      setExpanded(true);
      await animateMorph(token, true);
      if (token !== gen.current) return;
      setPanelActive(true);
    } finally {
      if (token === gen.current) {
        busy.current = false;
        window.setTimeout(() => setTrayUiPaused(false), 500);
        // 若展开过程中目标尺寸已切到中转站，收尾再贴合一次
        if (expandedRef.current) {
          const t = {
            width: shellPanelWRef.current,
            height: shellPanelHRef.current,
          };
          liveExpanded.width = t.width;
          liveExpanded.height = t.height;
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
    if (!expandedRef.current && revealRef.current <= 0.01) {
      clearSessionPanel();
      return;
    }
    bumpIslandActivity();
    const token = ++gen.current;
    busy.current = true;
    setTrayUiPaused(true);
    try {
      // 先 leave：立刻关摄像头，再开收起动画
      setPanelActive(false);
      setExpanded(false);
      // 收回一开始就直角贴顶，与下拉同理
      morphingRef.current = true;
      // 收起目标宽按当前文案锁定，避免动画落到过期 liveCollapsed
      snapCollapsedFromBar();
      paintDom(sizeRef.current, revealRef.current);
      await animateMorph(token, false);
      if (token !== gen.current) return;
      await setBarHeight(ISLAND_BAR_H);
      lastWinH.current = winHeight(ISLAND_BAR_H);
      morphingRef.current = false;
      // 再测一次 + 立刻 setSize，避免 React style 仍停在展开宽导致壳/居中错位
      const settled = snapCollapsedFromBar();
      paintDom(settled, 0);
      setSize(settled);
      setReveal(0);
      // 拖入会话覆盖仅本次展开有效；收起后恢复用户「下拉内容」
      clearSessionPanel();
      if (
        searchModeRef.current ||
        isFileSearchPlugin(scenarioOwnerRef.current)
      ) {
        if (retainSearchModeRef.current) {
          retainSearchModeRef.current = false;
          searchModeRef.current = true;
          setSearchMode(true);
          clearSearchLeaveTimer();
          searchLeavingRef.current = false;
          setSearchLeaving(false);
          // 保留情景主人；折叠宽钉回搜索栏
          liveCollapsed.width = ISLAND_SEARCH_COLLAPSED_W;
          syncCollapsedIslandWidth(ISLAND_SEARCH_COLLAPSED_W);
          void focusIslandSearchInput();
        } else {
          // 展开态 Esc/再热键：壳已 morph 完，chrome 再播交接
          exitIslandSearchChrome({ animated: true });
        }
      }
    } finally {
      if (token === gen.current) {
        morphingRef.current = false;
        busy.current = false;
        // After collapse settle + bar_comp suppress window.
        window.setTimeout(() => setTrayUiPaused(false), 900);
        scheduleImmerse();
      }
    }
  }

  /** 拉高悬浮窗到展开高度（AppBar 高度不变）；拖拽前预热，避免裁切黑块 */
  async function ensureExpandedWindow(): Promise<void> {
    const islandH = liveExpanded.height;
    const targetWinH = winHeight(islandH);
    const actualWinH = readActualWinH();
    if (actualWinH >= targetWinH - 2) {
      lastWinH.current = targetWinH;
      return;
    }
    await raiseIslandWindow(islandH);
  }

  function onIslandPointerDown(e: ReactPointerEvent<HTMLDivElement>) {
    clickTrace(
      "fe-island",
      `pointerdown btn=${e.button} busy=${busy.current} expanded=${expandedRef.current}`,
    );
    if (expandedRef.current || busy.current) return;
    if (e.button !== 0) return;
    // 消息提示：仅「内联」横幅支持在主岛上左滑划掉；冲突叠层在独立胶囊上滑
    if (msgBannerRef.current) {
      const stackedConflict =
        searchModeRef.current ||
        Boolean(scenarioOwnerRef.current) ||
        expandedRef.current ||
        revealRef.current > 0.12;
      if (!stackedConflict) {
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
    }
    // 未配置下拉/常驻面板：不进入下拉手势（拖入仍走 openPluginSession）
    if (!canShellPullExpand()) return;
    const shellPid = resolveShellExpandPluginId();
    if (shellPid) syncLiveExpandedForPlugin(shellPid);
    clearSessionPanel();
    void refreshIslandPrefsFromDb().then(setIslandPrefsState);
    bumpIslandActivity();
    if (leaveShrinkTimer.current != null) {
      window.clearTimeout(leaveShrinkTimer.current);
      leaveShrinkTimer.current = null;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = {
      pointerId: e.pointerId,
      startY: e.clientY,
      lastY: e.clientY,
      moved: false,
      active: true,
      winReady: false,
    };
    setSpringing(false);
    pullingRef.current = true;
    setPulling(true);
    // 拖拽过程不跟手长高岛形（会与窗口抬高抢跑 → 闪裁）；
    // 松手后与点击相同走 openPluginSession / animateMorph。
    paintDom(collapsedNow(), 0);
    void ensureExpandedWindow().then(() => {
      const d = drag.current;
      if (!d || d.pointerId !== e.pointerId) return;
      d.winReady = actualWinHRef.current >= winHeight(liveExpanded.height) - 2;
    });
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
    if (Math.abs(e.clientY - d.startY) > CLICK_SLOP) d.moved = true;
    // 故意不 paintDom 长高：展开动画与点击共用 openPluginSession
  }

  /** 下拉手势结束：打开时与点击同一路径，避免跟手长高造成闪裁 */
  function finishPull(open: boolean) {
    drag.current = null;
    pullingRef.current = false;
    setPulling(false);
    setSpringing(false);
    const settled = snapCollapsedFromBar();
    paintDom(settled, 0);
    setSize(settled);
    setReveal(0);
    if (open && canShellPullExpand()) {
      clearSessionPanel();
      const pid = resolveShellExpandPluginId();
      if (pid) void openPluginSession(pid);
      return;
    }
    clearSessionPanel();
    void shrinkIslandWindow().then(() => scheduleImmerse());
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
      finishPull(true);
      return;
    }
    const dy = d.lastY - d.startY;
    finishPull(pullProgress(dy) >= PULL_OPEN);
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
      else if (canShellPullExpand()) {
        const pid = resolveShellExpandPluginId();
        if (pid) void openPluginSession(pid);
      }
    }
  }

  useEffect(() => {
    void (async () => {
      await setBarHeight(ISLAND_BAR_H);
      lastWinH.current = winHeight(ISLAND_BAR_H);
      const r = await applyBarMaterial();
      if (r.dark != null) setBarGlassDark(r.dark);
      if (r.kind) setMaterial(r.kind);
    })();
  }, [islandPrefs.barGlass]);

  // Desktop ↔ window flips: re-sync (desktop forces Win32 glass on).
  useEffect(() => {
    void applyBarMaterial().then((r) => {
      if (r.dark != null) setBarGlassDark(r.dark);
      if (r.kind) setMaterial(r.kind);
    });
  }, [ambient.hwnd]);

  useEffect(() => {
    let unSettings: (() => void) | undefined;
    void listen("settings-closed", () => {
      bumpIslandActivity();
      window.dispatchEvent(new CustomEvent("wh-chrome-tokens"));
    }).then((fn) => {
      unSettings = fn;
    });
    return () => {
      unSettings?.();
    };
  }, []);

  useEffect(() => {
    let unMat: (() => void) | undefined;
    void listen<GlassPrefs>("material-prefs", (ev) => {
      void (async () => {
        try {
          const dark = await syncGlassCss(ev.payload);
          setBarGlassDark(dark);
          const kind = await invoke<string>("apply_window_effect", {});
          setMaterial((kind as Material) || "mica-alt");
        } catch {
          /* noop */
        }
      })();
    }).then((fn) => {
      unMat = fn;
    });
    const unDark = subscribeSystemDark(() => {
      void invoke<GlassPrefs>("get_material_prefs")
        .then((prefs) => {
          if (prefs.dark != null) return;
          return syncGlassCss(prefs).then((dark) => {
            setBarGlassDark(dark);
            return invoke<string>("apply_window_effect", {});
          });
        })
        .then((kind) => {
          if (kind) setMaterial((kind as Material) || "mica-alt");
        })
        .catch(() => undefined);
    });
    return () => {
      unMat?.();
      unDark();
    };
  }, []);

  // React 每次 commit 可能用 style={{width:size.width}} 盖掉 paintDom 的即时宽；
  // adaptive 歌词变宽时须按 sizeRef 重刷，并保持 island-beam 的 left:50% + translate 居中。
  useLayoutEffect(() => {
    const root = islandRef.current;
    if (root) {
      root.style.left = "50%";
      root.style.right = "auto";
      root.style.setProperty("translate", "-50% 0");
    }
    paintDom(sizeRef.current, revealRef.current);
  });

  useEffect(() => {
    if (expanded || busy.current || pulling || springing) return;
    // 托盘改为独立弹窗，主顶栏保持折叠高度
    if (!trayOpen) void setBarHeight(ISLAND_BAR_H);
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

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        // Prefer event stream; invoke is non-blocking (no foreign BitBlt).
        const first = await invoke<Ambient>("sample_ambient_color");
        if (!cancelled) setAmbient(first);
      } catch {
        /* noop */
      }
      try {
        unlisten = await listen<Ambient>("ambient-color", (ev) => {
          setAmbient(ev.payload);
        });
      } catch {
        /* noop */
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const onDesktop = (ambient.hwnd ?? 0) === 0;
    const desktopGlass = onDesktop && islandPrefs.barGlass;
    // 桌面 Win32 材质会混入主题 tint，反色按「壁纸×材质」感知色，避免跟纯壁纸采样偏差
    const mixGlass = (rgb: Rgb): Rgb => {
      if (!desktopGlass) return rgb;
      const tint = barGlassDark
        ? { r: 28, g: 28, b: 30 }
        : { r: 245, g: 245, b: 250 };
      const t = 0.42;
      return {
        r: Math.round(rgb.r * (1 - t) + tint.r * t),
        g: Math.round(rgb.g * (1 - t) + tint.g * t),
        b: Math.round(rgb.b * (1 - t) + tint.b * t),
      };
    };
    const fallback = mixGlass({ r: ambient.r, g: ambient.g, b: ambient.b });

    void (async () => {
      let left = fallback;
      let center = fallback;
      let right = fallback;
      if (ambient.png_base64 && (ambient.width ?? 0) > 1) {
        const bands = await sampleStripBands(ambient.png_base64);
        if (bands) {
          left = mixGlass(bands.left);
          center = mixGlass(bands.center);
          right = mixGlass(bands.right);
        }
      }
      if (cancelled) return;
      // 桌面 Win32 顶栏材质整条一致 — 左右勿再按壁纸分段反色
      if (desktopGlass) {
        const tokens = chromeTokens(center);
        setChromeLeft(tokens);
        setChromeCenter(tokens);
        setChromeRight(tokens);
        return;
      }
      setChromeLeft(chromeTokens(left));
      setChromeCenter(chromeTokens(center));
      setChromeRight(chromeTokens(right));
    })();

    return () => {
      cancelled = true;
    };
  }, [
    ambient.r,
    ambient.g,
    ambient.b,
    ambient.hwnd,
    ambient.png_base64,
    ambient.width,
    barGlassDark,
    islandPrefs.barGlass,
  ]);

  // After chrome CSS vars commit — shortcuts iframes mirror --chrome-left-* by hand.
  useEffect(() => {
    window.dispatchEvent(new CustomEvent("wh-chrome-tokens"));
  }, [chromeLeft.fg, chromeLeft.shadow, chromeRight.fg, chromeCenter.fg]);

  const sessionOverrideActive = Boolean(
    panelSessionArmedRef.current &&
      panelOverride &&
      panelOverride === panelSessionRef.current &&
      (expanded || pulling || reveal > 0.12 || busy.current),
  );
  const resolvedPullContent = resolveIslandPullContent({
    scenarioOwner,
    scenarioPull,
    sessionOverride: panelSessionRef.current,
    sessionOverrideActive,
    pullContent: islandPrefs.pullContent,
  }, enabledPullContent);

  /** 当前会话 / 投放 / 情景插件：同步面板壳尺寸 */
  const sizePluginId = parsePluginPanelId(resolvedPullContent) ?? dropPluginId;

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
    const unsubReg = pluginRegistry.subscribe(() => apply(null));
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
      unsubReg();
      unlisten?.();
    };
  }, [sizePluginId]);

  // Unified boot gate: HWND hidden until reveal; FE opacity as backup.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const reveal = () => {
      if (cancelled) return;
      setBootReady(true);
      void invoke<Ambient>("sample_ambient_color")
        .then((a) => {
          if (!cancelled && a) setAmbient(a);
        })
        .catch(() => undefined);
    };
    void invoke<boolean>("is_host_boot_ready")
      .then((ready) => {
        if (!cancelled && ready) reveal();
      })
      .catch(() => undefined);
    void listen<{ trayTotal?: number; trayClickable?: number; elapsedMs?: number }>(
      "host-boot-ready",
      (ev) => {
        if (cancelled) return;
        console.info("[boot] host-boot-ready", ev.payload);
        reveal();
      },
    ).then((fn) => {
      unlisten = fn;
    });
    const safety = window.setTimeout(() => {
      if (cancelled) return;
      console.warn("[boot] host-boot-ready timeout — revealing chrome");
      reveal();
    }, 8_000);
    return () => {
      cancelled = true;
      unlisten?.();
      window.clearTimeout(safety);
    };
  }, []);

  useEffect(() => {
    void hydrateIslandPrefs().then((prefs) => {
      setIslandPrefsState(prefs);
      clearStalePanelOverride();
    });
    const unsub = subscribeIslandPrefs(setIslandPrefsState);
    const unsubBus = islandNotifyBus.subscribe((b) => {
      if (!b) {
        msgBannerRef.current = null;
        setMsgBanner(null);
        return;
      }
      if (expandedRef.current || revealRef.current > 0.05) {
        console.info("[notify-bus] banner held (island busy)", b.id);
        return;
      }
      applyMsgBannerFromBus(b);
    });
    let unlistenPrefs: (() => void) | undefined;
    let unlistenTrayPrefs: (() => void) | undefined;
    let unlistenAttn: (() => void) | undefined;
    let unlistenTrayIcons: (() => void) | undefined;
    let unlistenPluginNotify: (() => void) | undefined;
    let unlistenStaging: (() => void) | undefined;
    let unlistenBar: (() => void) | undefined;
    let unlistenScenario: (() => void) | undefined;
    let unlistenSession: (() => void) | undefined;
    let unlistenSearchHotkey: (() => void) | undefined;
    let unsubPlugins = () => {};
    void bootstrapPlugins();
    void subscribeInstalledPlugins().then((fn) => {
      unsubPlugins = fn;
    });
    void listen<IslandPrefs>("island-prefs", (ev) => {
      const prev = islandPrefsRef.current;
      const next = applyIslandPrefsSnapshot(ev.payload);
      setIslandPrefsState(next);
      if (next.pullContent !== prev.pullContent) {
        const sessPid = parsePluginPanelId(panelSessionRef.current ?? "");
        const wantPid = parsePluginPanelId(
          enabledPullContent(next.pullContent),
        );
        if (sessPid && sessPid !== wantPid) clearSessionPanel();
      }
      clearStalePanelOverride();
    }).then((fn) => {
      unlistenPrefs = fn;
    });
    void invoke<{ flash_notify?: Record<string, boolean> }>("get_tray_prefs")
      .then((p) => {
        trayFlashNotifyRef.current = p.flash_notify ?? {};
      })
      .catch(() => undefined);
    void listen<{ flash_notify?: Record<string, boolean> }>("tray-prefs", (ev) => {
      trayFlashNotifyRef.current = ev.payload.flash_notify ?? {};
      const cur = islandNotifyBus.getCurrent();
      if (
        cur?.source === "tray" &&
        !trayFlashNotifyAllowed({
          id: cur.tray?.iconId,
          pinKey: cur.tray?.pinKey,
        })
      ) {
        islandNotifyBus.dismiss(cur.id);
      }
      // Sweep queue for newly muted icons
      for (const [k, v] of Object.entries(trayFlashNotifyRef.current)) {
        if (v === false) {
          islandNotifyBus.dismissTrayIcon({ pinKey: k, iconId: k });
        }
      }
    }).then((fn) => {
      unlistenTrayPrefs = fn;
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
        setOverlayBar((prev) => {
          if (prev?.pluginId !== pluginId) return prev;
          overlayBarRef.current = null;
          return null;
        });
        if (panelSessionRef.current === `plugin:${pluginId}`) {
          clearSessionPanel();
          // 下拉为「无」：清空后直接收起，勿落回空面板
          if (!hasConfiguredPullContent() && expandedRef.current) {
            queueMicrotask(() => {
              if (expandedRef.current) void collapse();
            });
          }
        }
        // 常驻+下拉皆无：清空后立刻沉浸，勿留黑色空岛
        queueMicrotask(() => {
          if (!expandedRef.current) scheduleImmerse();
        });
        return;
      }
      const nextBar = {
        pluginId,
        text: formatStagingBarText(rec.manifest.name || pluginId, summary),
        title: rec.manifest.name || pluginId,
      };
      overlayBarRef.current = nextBar;
      setOverlayBar(nextBar);
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
      const residentId = islandPrefsRef.current.barResident;
      const rec = pluginRegistry.get(p.pluginId);
      const tempOnly = Boolean(rec?.manifest.slots?.["island.bar"]?.excludeFromBarResident);
      const isScenarioOwner = scenarioOwnerRef.current === p.pluginId;
      const hasScenario = Boolean(rec?.manifest.slots?.["island.scenario"]);
      const adaptive = resolveIslandBarAdaptive(p.pluginId).enabled;

      // Visible layer for DOM paint: overlay > scenario > resident
      // 搜索 chrome 接管岛栏时禁止再把天气等常驻文案刷进 DOM
      const paintVisibleBar = (text: string, pluginId: string | null, showDot: boolean) => {
        if (hostSearchLocksScenario()) return;
        if (overlayBarRef.current) return;
        if (barStagingTextRef.current) {
          barStagingTextRef.current.textContent = text;
        }
        if (!text.trim()) {
          if (!scenarioBarRef.current && !residentBarRef.current) {
            syncCollapsedIslandWidthRef.current(ISLAND_COLLAPSED_W_DEFAULT);
          }
          return;
        }
        syncCollapsedIslandWidthRef.current(
          widthForBarLabelRef.current(text, pluginId, showDot, false),
        );
      };

      // 情景层：claim 主人，或 scenario 槽插件 setBar 时自动晋升（避免 claim/setBar 竞态）
      // Host Alt+空格搜索锁：禁止其它情景靠 setBar 抢主人（正在播放歌词会高频抢）
      if (hasScenario && (isScenarioOwner || next)) {
        if (next && !scenarioGateAllows(p.pluginId)) {
          if (isScenarioOwner && !hostSearchLocksScenario()) clearScenarioLayer();
          return;
        }
        if (next && scenarioOwnerRef.current !== p.pluginId) {
          if (hostSearchLocksScenario() && !isFileSearchPlugin(p.pluginId)) {
            return;
          }
          scenarioOwnerRef.current = p.pluginId;
          setScenarioOwner(p.pluginId);
          setScenarioPull(`plugin:${p.pluginId}`);
          if (isFileSearchPlugin(p.pluginId)) {
            searchModeRef.current = true;
            setSearchMode(true);
          }
        }
        if (
          hostSearchLocksScenario() &&
          !isFileSearchPlugin(p.pluginId) &&
          scenarioOwnerRef.current !== p.pluginId
        ) {
          return;
        }
        const prev = scenarioBarRef.current;
        if (adaptive && next && prev && prev.pluginId === next.pluginId) {
          scenarioBarRef.current = next;
          paintVisibleBar(next.text, next.pluginId, false);
          return;
        }
        if (adaptive && !next && prev) {
          scenarioBarRef.current = null;
          setScenarioBar(null);
          const fallback = residentBarRef.current;
          paintVisibleBar(fallback?.text ?? "", fallback?.pluginId ?? null, false);
          return;
        }
        setScenarioBar(next);
        return;
      }
      if (isScenarioOwner && !next) {
        if (hostSearchLocksScenario() && isFileSearchPlugin(p.pluginId)) {
          // 搜索锁下忽略文件搜索空 setBar，避免冲掉 Host chrome
          return;
        }
        setScenarioBar(null);
        const fallback = residentBarRef.current;
        paintVisibleBar(fallback?.text ?? "", fallback?.pluginId ?? null, false);
        return;
      }

      // 常驻层
      if (residentId && p.pluginId === residentId) {
        const prev = residentBarRef.current;
        if (adaptive && next && prev && prev.pluginId === next.pluginId) {
          residentBarRef.current = next;
          if (!scenarioBarRef.current) {
            paintVisibleBar(next.text, next.pluginId, false);
          }
          return;
        }
        if (adaptive && !next && prev) {
          residentBarRef.current = null;
          setResidentBar(null);
          if (!scenarioBarRef.current) {
            paintVisibleBar("", null, false);
          }
          return;
        }
        setResidentBar(next);
        return;
      }

      // 临时层：中转站等
      if (tempOnly) {
        setOverlayBar((prev) => {
          if (cleared) return prev?.pluginId === p.pluginId ? null : prev;
          return next;
        });
        return;
      }
    }).then((fn) => {
      unlistenBar = fn;
    });
    void listen<{ action?: string; pluginId?: string }>("island-scenario", (ev) => {
      const action = ev.payload?.action;
      const pluginId = typeof ev.payload?.pluginId === "string" ? ev.payload.pluginId : "";
      if (!pluginId) return;
      if (action === "claim") {
        // Host 搜索锁：其它情景（正在播放等）不得抢 Alt+空格主人
        if (hostSearchLocksScenario() && !isFileSearchPlugin(pluginId)) {
          return;
        }
        if (!scenarioGateAllows(pluginId)) {
          if (!hostSearchLocksScenario()) clearScenarioLayer();
          return;
        }
        scenarioOwnerRef.current = pluginId;
        setScenarioOwner(pluginId);
        setScenarioPull(`plugin:${pluginId}`);
        // 文件搜索 claim = 立即接管岛栏为搜索框（与 Host Alt+空格同源）
        if (isFileSearchPlugin(pluginId)) {
          searchModeRef.current = true;
          setSearchMode(true);
          liveCollapsed.width = ISLAND_SEARCH_COLLAPSED_W;
          syncCollapsedIslandWidth(ISLAND_SEARCH_COLLAPSED_W);
          queueMicrotask(() => {
            void focusIslandSearchInput();
          });
        } else if (!searchModeRef.current) {
          setScenarioBar((prev) => (prev?.pluginId === pluginId ? prev : null));
        }
        return;
      }
      if (action === "release") {
        if (scenarioOwnerRef.current !== pluginId) return;
        // 搜索锁下忽略非文件搜索的 release（防止正在播放 release 清掉搜索情景）
        if (hostSearchLocksScenario() && !isFileSearchPlugin(pluginId)) {
          return;
        }
        if (isFileSearchPlugin(pluginId)) {
          // 仅插件主动 release：若 Host 仍在搜索锁中，保持 chrome，只同步层
          if (searchModeRef.current) {
            return;
          }
          searchModeRef.current = false;
          setSearchMode(false);
          setSearchDraft("");
          setSearchSubmit(null);
        }
        clearScenarioLayer();
      }
    }).then((fn) => {
      unlistenScenario = fn;
    });
    void listen<{ action?: string; pluginId?: string }>("island-session", (ev) => {
      const action = ev.payload?.action;
      const pluginId = ev.payload?.pluginId;
      if (action === "open" && pluginId) {
        armPluginSession(pluginId);
        if (!expandedRef.current) {
          void expand({ force: true }).then(() => {
            if (!expandedRef.current) clearSessionPanel();
          });
        }
      } else if (action === "close") {
        clearSessionPanel();
        if (expandedRef.current) void collapse();
      }
    }).then((fn) => {
      unlistenSession = fn;
    });
    void listen<{
      action?: string;
      pluginId?: string | null;
      id?: string;
    }>("hotkey-action", (ev) => {
      const action = ev.payload?.action ?? "";
      console.info("[hotkey-action]", action, ev.payload?.id);
      if (action === "island.search.toggle") {
        void toggleIslandSearchHotkeyRef.current?.();
        return;
      }
      if (
        action === "fileSearch.openFavorites" ||
        (ev.payload?.pluginId?.replace(/__dev$/, "") ===
          ISLAND_SEARCH_PLUGIN_ID &&
          action === "openFavorites")
      ) {
        void openFavoritesHotkeyRef.current?.();
      }
    }).then((fn) => {
      unlistenSearchHotkey = fn;
    });
    if (TRAY_UI_ENABLED) {
      void listen<TrayAttention>("tray-attention", (ev) => {
        console.info("[tray-attention] event", ev.payload?.id, ev.payload?.tooltip);
        showMsgBanner(ev.payload);
      }).then((fn) => {
        unlistenAttn = fn;
      });
      type TrayIconFlash = {
        id: string;
        pin_key?: string;
        tooltip: string;
        process: string;
        icon_png_base64: string;
        hwnd: number;
        uid: number;
        callback_msg: number;
        version?: number;
        flashing?: boolean;
        system_tray?: boolean;
      };
      const syncPresenceFromTrays = (icons: TrayIconFlash[]) => {
        refreshPresenceKeys(icons, undefined);
        syncFlashingTrayBanner(icons);
      };
      void listen<TrayIconFlash[]>("tray-icons", (ev) => {
        syncPresenceFromTrays(ev.payload ?? []);
      }).then((fn) => {
        unlistenTrayIcons = fn;
      });
      void invoke<TrayIconFlash[]>("list_tray_icons")
        .then((icons) => syncPresenceFromTrays(icons ?? []))
        .catch(() => undefined);
    }

    let unlistenWindows: (() => void) | undefined;
    const syncWindows = (list: WindowInfo[]) => {
      refreshPresenceKeys(undefined, list);
    };
    void listen<{ windows?: WindowInfo[] }>("hub-windows-changed", (ev) => {
      const list = ev.payload?.windows;
      if (Array.isArray(list)) syncWindows(list);
    }).then((fn) => {
      unlistenWindows = fn;
    });
    void invoke<WindowInfo[]>("list_open_windows")
      .then((list) => syncWindows(list ?? []))
      .catch(() => undefined);
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
      unlistenTrayPrefs?.();
      unlistenAttn?.();
      unlistenTrayIcons?.();
      unlistenWindows?.();
      unlistenPluginNotify?.();
      unlistenStaging?.();
      unlistenBar?.();
      unlistenScenario?.();
      unlistenSession?.();
      unlistenSearchHotkey?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Prefs 变更（含 scenarioGates）时复查当前情景主人
  useEffect(() => {
    const owner = scenarioOwnerRef.current;
    if (owner && !scenarioPresenceOk(
      owner,
      islandPrefs.scenarioGates,
      liveTrayKeysRef.current,
      liveWindowKeysRef.current,
    )) {
      clearScenarioLayer();
    }
  }, [islandPrefs.scenarioGates]);

  useEffect(() => {
    // 收起后：只补 bus 上已有横幅，禁止 sync list_tray_icons（PNG 风暴叠 bar_comp → HUNG）
    if (expanded || reveal > 0.05) return;
    if (!TRAY_UI_ENABLED) return;
    const pending = islandNotifyBus.getCurrent();
    if (pending && !msgBannerRef.current) {
      applyMsgBannerFromBus(pending);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expanded, reveal]);

  useEffect(() => {
    // 展开 / 拖放高亮 / 通知：暂停沉浸。中转站有内容不阻断。
    // 独立托盘弹窗不再退出沉浸（与左侧状态菜单一致，保持常驻透底）。
    if (expanded || pulling || springing || reveal > 0.02 || msgBanner || dropTarget || showSearchChrome) {
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
    pulling,
    springing,
    reveal,
    msgBanner,
    dropTarget,
    showSearchChrome,
    // 只看「有谁占栏」，勿依赖文案：歌词 setBar 每秒变 text 会反复清/排 immerse 定时器 → 整机卡
    overlayBar?.pluginId,
    scenarioBar?.pluginId,
    residentBar?.pluginId,
    islandPrefs.autoImmerse,
    islandPrefs.immerseIdleSec,
    islandPrefs.pullContent,
    islandPrefs.barResident,
    immersed,
  ]);

  /** 关闭「闪动时通知上岛」：立刻撤掉岛上的托盘消息横幅（托盘图标仍可继续闪） */
  useEffect(() => {
    if (islandPrefs.msgNotify) return;
    islandNotifyBus.dismissSource("tray");
    const banner = msgBannerRef.current;
    if (!banner || banner.source !== "tray") return;
    const el = notifyRef.current;
    if (el) {
      el.style.transition = "";
      el.style.transform = "";
      el.style.opacity = "";
    }
    const id = banner.notifyId;
    msgBannerRef.current = null;
    setMsgBanner(null);
    if (id) islandNotifyBus.dismiss(id);
    else islandNotifyBus.dismiss();
    scheduleImmerse();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [islandPrefs.msgNotify]);

  const effectivePullContent =
    resolvedPullContent ||
    (sessionOverrideActive ? panelOverride : null) ||
    "";
  const panelHostPluginId =
    parsePluginPanelId(effectivePullContent || panelOverride || "") ||
    "none";
  /**
   * 主岛已被临时占用时，通知不得盖住栏内内容，改为下方独立胶囊：
   * Alt+空格搜索 / 情景临时（正在播放等）/ 下拉展开。
   */
  const notifyConflict =
    showSearchChrome ||
    Boolean(scenarioOwner) ||
    expanded ||
    pulling ||
    reveal > 0.12;
  const notifyStacked = Boolean(msgBanner) && notifyConflict;
  const notifyInline = Boolean(msgBanner) && !notifyConflict;
  const weatherBarExiting =
    notifyInline || (searchActive && !searchLeaving);
  const stagingBar = islandBar?.text ?? "";
  const dropPluginName = dropPluginId
    ? pluginRegistry.get(dropPluginId)?.manifest.name?.trim() || "中转站"
    : "";
  /** 拖入时临时占满岛栏文案（无常驻时也能看见「松开存入」提示） */
  const barText =
    dropTarget && dropPluginId ? `${dropPluginName}|松开存入` : stagingBar;
  const barPluginId = dropTarget && dropPluginId ? dropPluginId : islandBar?.pluginId;
  const barTitle =
    dropTarget && dropPluginId
      ? `${dropPluginName} · 松开存入`
      : islandBar?.title || stagingBar || "打开面板";
  /** 绿点仅中转站等临时摘要 / 拖放提示；情景临时与常驻摘要不要点 */
  const showBarStagingDot =
    dropTarget ||
    Boolean(
      overlayBar &&
        barPluginId &&
        pluginRegistry.get(barPluginId)?.manifest.slots?.["island.bar"]
          ?.excludeFromBarResident,
    );

  // 冲突通知叠层：拉高/收回窗口附加高度（不改 SVG 岛身尺寸）
  useLayoutEffect(() => {
    const extra = notifyStacked ? NOTIFY_STACK_GAP + NOTIFY_STACK_H : 0;
    if (liveNotifyStackExtra === extra) return;
    liveNotifyStackExtra = extra;
    const islandH = Math.max(ISLAND_BAR_H, sizeRef.current.height);
    lastWinH.current = winHeight(islandH);
    void setBarHeight(islandH);
  }, [notifyStacked]);

  // 叠层胶囊：主窗仍是全屏宽，左右透明条带必须 OS 级穿透。
  // CSS pointer-events 不够时，按光标是否落在可点区域切换 ignoreCursorEvents。
  useEffect(() => {
    const win = getCurrentWindow();
    const passThrough =
      notifyStacked && !expanded && !pulling && reveal <= 0.12;
    if (!passThrough) {
      void win.setIgnoreCursorEvents(false).catch(() => undefined);
      return;
    }

    let cancelled = false;
    let lastIgnore: boolean | null = null;
    const pad = 2;

    const hit = (r: DOMRect | undefined, x: number, y: number) => {
      if (!r || r.width < 1 || r.height < 1) return false;
      return (
        x >= r.left - pad &&
        x <= r.right + pad &&
        y >= r.top - pad &&
        y <= r.bottom + pad
      );
    };

    const sync = async () => {
      if (cancelled) return;
      try {
        const pos = await invoke<[number, number] | null>("main_cursor_client_pos");
        if (!pos || cancelled) return;
        const [x, y] = pos;
        const over =
          hit(notifyRef.current?.getBoundingClientRect(), x, y) ||
          hit(islandRef.current?.getBoundingClientRect(), x, y) ||
          hit(settingsAnchorRef.current?.getBoundingClientRect(), x, y) ||
          hit(
            document.querySelector(".shortcuts-host:not(.is-empty)")?.getBoundingClientRect(),
            x,
            y,
          ) ||
          hit(document.querySelector(".tray-cluster")?.getBoundingClientRect(), x, y);
        const ignore = !over;
        if (lastIgnore === ignore) return;
        lastIgnore = ignore;
        await win.setIgnoreCursorEvents(ignore);
      } catch {
        /* noop */
      }
    };

    const id = window.setInterval(() => {
      void sync();
    }, 32);
    void sync();
    return () => {
      cancelled = true;
      window.clearInterval(id);
      void win.setIgnoreCursorEvents(false).catch(() => undefined);
    };
  }, [notifyStacked, expanded, pulling, reveal]);

  // 岛栏折叠宽自适应：slots.island.bar.adaptiveWidth（如正在播放长歌词）
  useLayoutEffect(() => {
    if (
      expanded ||
      pulling ||
      springing ||
      reveal > 0.02 ||
      notifyInline ||
      showSearchChrome
    ) {
      if (searchActive && !searchLeaving && !expanded) {
        syncCollapsedIslandWidth(ISLAND_SEARCH_COLLAPSED_W);
      }
      return;
    }
    const text =
      dropTarget && dropPluginId
        ? `${dropPluginName}|松开存入`
        : (overlayBar?.text ?? residentBarRef.current?.text ?? barText);
    const nextW = widthForBarLabel(text, barPluginId, showBarStagingDot, dropTarget);
    if (Math.abs(nextW - liveCollapsed.width) < 2) return;
    syncCollapsedIslandWidth(nextW);
  }, [
    barText,
    overlayBar,
    residentBar,
    barPluginId,
    showBarStagingDot,
    dropTarget,
    dropPluginId,
    dropPluginName,
    expanded,
    pulling,
    springing,
    reveal,
    msgBanner,
    showSearchChrome,
    searchActive,
    searchLeaving,
    notifyInline,
  ]);

  // 文案以 ref 为准（adaptive 高频路径不 setState）；其它重渲染后对齐 DOM
  // 搜索激活时禁止把天气常驻刷回 span；离场动画期间允许刷回以便交接
  useLayoutEffect(() => {
    if (searchModeRef.current && !searchLeavingRef.current) return;
    if (
      isFileSearchPlugin(scenarioOwnerRef.current) &&
      !searchLeavingRef.current
    ) {
      return;
    }
    const el = barStagingTextRef.current;
    if (!el) return;
    const text =
      dropTarget && dropPluginId
        ? `${dropPluginName}|松开存入`
        : (overlayBar ?? scenarioBar ?? residentBar)?.text ?? "";
    if (el.textContent !== text) el.textContent = text;
  }, [
    overlayBar,
    scenarioBar,
    residentBar,
    dropTarget,
    dropPluginId,
    dropPluginName,
    searchMode,
    scenarioOwner,
    searchLeaving,
  ]);

  const sizingPluginId = resolvePanelSizingPluginId();
  const viewW = sizingPluginId ? shellPanelWRef.current : VIEW_W_DEFAULT;
  const viewH = sizingPluginId ? shellPanelHRef.current : VIEW_H_DEFAULT;
  const pluginStagingShell =
    !!sizingPluginId && isStagingPanelShell(viewW, viewH);
  if (!busy.current && !morphingRef.current) {
    liveExpanded.width = viewW;
    liveExpanded.height = viewH;
  }

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
    const targetW = sizingPluginId ? shellPanelWRef.current : VIEW_W_DEFAULT;
    const targetH = sizingPluginId ? shellPanelHRef.current : VIEW_H_DEFAULT;
    if (!busy.current && !morphingRef.current) {
      liveExpanded.width = targetW;
      liveExpanded.height = targetH;
    }
    if (!expanded) return;
    if (busy.current || morphingRef.current) return;
    // 无会话、也无可用下拉内容 → 收起，避免空壳面板
    if (!sizingPluginId) {
      void collapse();
      return;
    }
    // 搜索会话未主动关闭：禁止 morph 把已展开面板缩回小尺寸
    if (
      searchModeRef.current &&
      (targetW < sizeRef.current.width - 2 ||
        targetH < sizeRef.current.height - 2)
    ) {
      return;
    }
    morphExpandedSize({ width: targetW, height: targetH });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sizingPluginId, shellPanelW, shellPanelH, expanded]);

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
    panelSessionRef.current = `plugin:${pluginId}`;
    panelSessionArmedRef.current = true;
    setPanelOverride(`plugin:${pluginId}`);
  }

  async function openPluginSession(pluginId: string | null | undefined) {
    if (!pluginId || !pluginRegistry.get(pluginId)?.enabled) return;
    armPluginSession(pluginId);
    if (!expandedRef.current) {
      await expand({ force: true });
      if (!expandedRef.current) clearSessionPanel();
    }
  }

  async function focusIslandSearchInput() {
    /** DOM 聚焦重试（等 React 提交搜索 input） */
    const attemptDomFocus = () => {
      const el = searchInputRef.current;
      if (!el) return false;
      el.focus({ preventScroll: true });
      el.select();
      return document.activeElement === el;
    };
    const scheduleDomFocus = () => {
      queueMicrotask(() => attemptDomFocus());
      for (const ms of [0, 50, 120, 250, 400]) {
        window.setTimeout(() => attemptDomFocus(), ms);
      }
    };
    try {
      await invoke("float_overlay");
      await invoke("activate_main_island");
    } catch {
      /* noop outside tauri */
    }
    scheduleDomFocus();
  }

  function clearSearchLeaveTimer() {
    if (searchLeaveTimerRef.current != null) {
      window.clearTimeout(searchLeaveTimerRef.current);
      searchLeaveTimerRef.current = null;
    }
  }

  /** 立刻卸掉搜索逻辑态（情景 / mode）；draft 可延后清以免离场闪空 */
  function clearSearchChromeState(opts?: { clearDraft?: boolean }) {
    searchModeRef.current = false;
    setSearchMode(false);
    if (opts?.clearDraft !== false) {
      setSearchDraft("");
      setSearchSubmit(null);
    }
    clearSessionPanel();
    if (isFileSearchPlugin(scenarioOwnerRef.current)) {
      const searchPid = scenarioOwnerRef.current;
      clearScenarioLayer();
      if (searchPid) {
        void invoke("hub_island_release_scenario", {
          pluginId: searchPid,
        }).catch(() => undefined);
      }
    }
  }

  /**
   * 退出 Alt+空格搜索 chrome。
   * animated：搜索框上滑淡出、常驻摘要回弹（Esc / Alt+空格折叠态）。
   */
  function exitIslandSearchChrome(opts?: { animated?: boolean }) {
    const active =
      searchModeRef.current ||
      isFileSearchPlugin(scenarioOwnerRef.current) ||
      searchLeavingRef.current;
    if (!active) return;

    const reduceMotion =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
    const animated = opts?.animated !== false && !reduceMotion;

    if (!animated) {
      clearSearchLeaveTimer();
      searchLeavingRef.current = false;
      setSearchLeaving(false);
      clearSearchChromeState({ clearDraft: true });
      snapCollapsedFromBar();
      syncCollapsedIslandWidth(liveCollapsed.width);
      return;
    }

    if (searchLeavingRef.current) return;
    searchLeavingRef.current = true;
    setSearchLeaving(true);
    // 先清逻辑态，让天气文案可刷回；DOM 仍因 searchLeaving 保留搜索框播离场
    clearSearchChromeState({ clearDraft: false });
    queueMicrotask(() => {
      snapCollapsedFromBar();
      syncCollapsedIslandWidth(liveCollapsed.width);
    });
    clearSearchLeaveTimer();
    searchLeaveTimerRef.current = window.setTimeout(() => {
      searchLeaveTimerRef.current = null;
      searchLeavingRef.current = false;
      setSearchLeaving(false);
      setSearchDraft("");
      setSearchSubmit(null);
    }, SEARCH_CHROME_EXIT_MS);
  }

  async function enterIslandSearchMode() {
    if (!arePluginsReady()) {
      await bootstrapPlugins();
    }
    const pluginId = resolveIslandSearchPluginId();
    if (!pluginId) return;

    clearSearchLeaveTimer();
    searchLeavingRef.current = false;
    setSearchLeaving(false);

    if (!islandSearchScenarioClaimOk(pluginId, scenarioGateAllows)) return;

    // 热键路径：先置顶/取消点击穿透并抢焦点，再 expand（避免「要先摸一下顶栏才出来」）
    try {
      await getCurrentWindow().setIgnoreCursorEvents(false);
    } catch {
      /* noop */
    }
    void focusIslandSearchInput();

    bumpIslandActivity();
    searchModeRef.current = true;
    setSearchMode(true);
    immersedRef.current = false;
    setImmersed(false);
    setSearchDraft("");
    setSearchSubmit(null);
    liveCollapsed.width = ISLAND_SEARCH_COLLAPSED_W;

    scenarioOwnerRef.current = pluginId;
    setScenarioOwner(pluginId);
    setScenarioPull(`plugin:${pluginId}`);
    setScenarioBar({
      pluginId,
      text: " ",
      title: "Alt+空格 · Everything",
    });
    try {
      await invoke("hub_island_claim_scenario", { pluginId });
    } catch (err) {
      console.warn("[island-search] claimScenario", err);
    }
    armPluginSession(pluginId);

    // 立刻展开；顶栏仍用原 Host 搜索框（panel 只画最近使用网格）
    if (!expandedRef.current) {
      await expand({ force: true });
    }
    void focusIslandSearchInput();
  }

  async function toggleIslandSearchMode() {
    if (searchToggleBusyRef.current) return;
    // 热键连按：勿在 toggle 层 await bootstrap，交给 enterIslandSearchMode
    // 离场动画中再按热键 → 取消离场并重新进入（避免「时好时坏」被吞）
    if (searchLeavingRef.current) {
      if (!resolveIslandSearchPluginId()) return;
      clearSearchLeaveTimer();
      searchLeavingRef.current = false;
      setSearchLeaving(false);
      searchToggleBusyRef.current = true;
      try {
        await enterIslandSearchMode();
      } finally {
        searchToggleBusyRef.current = false;
      }
      return;
    }
    if (searchModeRef.current) {
      if (expandedRef.current) {
        // 下拉已开：收起并退出搜索（壳 morph；chrome 在 collapse 末尾清）
        retainSearchModeRef.current = false;
        void collapse();
      } else {
        exitIslandSearchChrome({ animated: true });
      }
      return;
    }
    if (!resolveIslandSearchPluginId()) return;
    searchToggleBusyRef.current = true;
    try {
      await enterIslandSearchMode();
    } finally {
      searchToggleBusyRef.current = false;
    }
  }
  toggleIslandSearchHotkeyRef.current = () => {
    void toggleIslandSearchMode();
  };

  async function openFileSearchFavorites() {
    const pluginId =
      resolveIslandFileSearchPluginId() || resolveIslandSearchPluginId();
    if (!pluginId) return;
    if (!islandSearchScenarioClaimOk(pluginId, scenarioGateAllows)) return;
    bumpIslandActivity();
    // Favorites always targets file-search
    clearSearchLeaveTimer();
    searchLeavingRef.current = false;
    setSearchLeaving(false);
    searchModeRef.current = true;
    setSearchMode(true);
    setSearchDraft("");
    scenarioOwnerRef.current = pluginId;
    setScenarioOwner(pluginId);
    setScenarioPull(`plugin:${pluginId}`);
    try {
      await invoke("hub_island_claim_scenario", { pluginId });
    } catch (err) {
      console.warn("[island-search] claimScenario favorites", err);
    }
    armPluginSession(pluginId);
    const fire = () =>
      setSearchSubmit({
        nonce: Date.now(),
        query: "",
        action: "openFavorites",
      });
    if (!expandedRef.current) {
      await expand({ force: true });
    }
    fire();
  }
  openFavoritesHotkeyRef.current = () => {
    void openFileSearchFavorites();
  };

  /** Launcher → 文件搜索（可选带 query） */
  async function handoffIslandFileSearch(query: string) {
    const pluginId = resolveIslandFileSearchPluginId();
    if (!pluginId) return;
    if (!islandSearchScenarioClaimOk(pluginId, scenarioGateAllows)) return;
    bumpIslandActivity();
    searchModeRef.current = true;
    setSearchMode(true);
    const q = String(query || "").trim();
    setSearchDraft(q);
    scenarioOwnerRef.current = pluginId;
    setScenarioOwner(pluginId);
    setScenarioPull(`plugin:${pluginId}`);
    try {
      await invoke("hub_island_claim_scenario", { pluginId });
    } catch (err) {
      console.warn("[island-search] handoff claim", err);
    }
    armPluginSession(pluginId);
    if (!expandedRef.current) {
      await expand({ force: true });
    }
    setSearchSubmit({
      nonce: Date.now(),
      query: q,
      action: q ? "submit" : "openFavorites",
    });
  }
  handoffFileSearchRef.current = (query: string) => {
    void handoffIslandFileSearch(query);
  };

  function submitIslandSearch() {
    const pluginId = resolveIslandSearchPluginId();
    const q = searchDraft.trim();
    bumpIslandActivity();
    if (pluginId) {
      armPluginSession(pluginId);
      if (scenarioOwnerRef.current !== pluginId) {
        scenarioOwnerRef.current = pluginId;
        setScenarioOwner(pluginId);
      }
      setScenarioPull(`plugin:${pluginId}`);
    }
    const fire = () =>
      setSearchSubmit({
        nonce: Date.now(),
        query: q,
        action: "submit",
      });
    if (!expandedRef.current) {
      void expand({ force: true }).then(() => {
        fire();
        queueMicrotask(() => searchInputRef.current?.focus());
      });
      return;
    }
    fire();
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
      for (let i = 0; i < files.length; i++) {
        const f = files.item(i);
        if (!f || !f.type.startsWith("image/")) continue;
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
    }
  }

  function onIslandDragEnter(e: ReactDragEvent) {
    if (!dropPluginIdRef.current) return;
    e.preventDefault();
    e.stopPropagation();
    bumpIslandActivity();
    setDropTarget(true);
    void invoke("float_overlay").catch(() => undefined);
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
    if (!expandedRef.current) {
      void invoke("settle_overlay").catch(() => undefined);
    }
  }

  async function onIslandDrop(e: ReactDragEvent) {
    const pluginId = dropPluginIdRef.current;
    if (!pluginId) return;
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(false);
    bumpIslandActivity();
    void invoke("float_overlay").catch(() => undefined);
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
          void invoke("float_overlay").catch(() => undefined);
        } else if (p.type === "leave") {
          setDropTarget(false);
          if (!expandedRef.current) {
            void invoke("settle_overlay").catch(() => undefined);
          }
        } else if (p.type === "drop") {
          setDropTarget(false);
          bumpIslandActivity();
          void invoke("float_overlay").catch(() => undefined);
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

  const ambientFromWindow = (ambient.hwnd ?? 0) !== 0;
  /** Win32 模糊仅桌面 + 开关；有窗口时始终只用吸色 */
  const barGlassOn = !ambientFromWindow && islandPrefs.barGlass;

  const ambientCss = {
    ["--island-top-gap" as string]: `${TOP_GAP}px`,
    ["--ambient-r" as string]: String(ambient.r),
    ["--ambient-g" as string]: String(ambient.g),
    ["--ambient-b" as string]: String(ambient.b),
    ...chromeCssVars("left", chromeLeft),
    ...chromeCssVars("center", chromeCenter),
    ...chromeCssVars("right", chromeRight),
  } as CSSProperties;

  const stripStyle: CSSProperties =
    ambient.png_base64 && (ambient.width ?? 0) > 1
      ? {
          // 整条边缘：色带铺满顶栏（offset=0 时用 100% 避免 DPI 缝）
          backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})`,
          backgroundImage: `url(data:image/png;base64,${ambient.png_base64})`,
          backgroundRepeat: "no-repeat",
          backgroundSize:
            ambient.offset_x === 0
              ? "100% 100%"
              : ambient.span_width && ambient.span_width > 0
                ? `${ambient.span_width}px 100%`
                : "100% 100%",
          backgroundPosition:
            ambient.offset_x === 0
              ? "0 0"
              : typeof ambient.offset_x === "number"
                ? `${ambient.offset_x}px 0`
                : "0 0",
        }
      : {
          // 仅取中间：整条纯色
          backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})`,
          backgroundImage: "none",
        };

  const shellExpanded = expanded || reveal > 0.2;

  function renderNotifyBanner(opts: { stacked: boolean }) {
    if (!msgBanner) return null;
    return (
      <div
        className={`bar-notify${opts.stacked ? " is-stacked" : ""}`}
        key={msgBanner.key}
        ref={notifyRef}
        role="button"
        tabIndex={0}
        data-notify-accent={msgBanner.accentColor || undefined}
        style={
          opts.stacked
            ? ({
                ["--notify-stack-accent" as string]:
                  msgBanner.accentColor || "#34c759",
              } as CSSProperties)
            : undefined
        }
        onPointerDown={(e) => {
          e.stopPropagation();
          if (!opts.stacked || e.button !== 0) return;
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
        }}
        onPointerMove={opts.stacked ? onIslandPointerMove : undefined}
        onPointerUp={opts.stacked ? onIslandPointerUp : undefined}
        onPointerCancel={opts.stacked ? onIslandPointerCancel : undefined}
        onClick={(e) => {
          e.stopPropagation();
          const banner = msgBannerRef.current;
          if (!banner) return;
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
        }}
      >
        <div className="bar-notify-slot is-start">
          {(() => {
            const act = actionsForSlot(msgBanner.actions, "start");
            if (!act) return null;
            return (
              <button
                type="button"
                className="bar-notify-action"
                style={{ background: act.background }}
                {...hostTipPointerProps(act.label || act.id)}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  void hideChromeHoverTip();
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
                {...hostTipPointerProps(act.label || act.id)}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  void hideChromeHoverTip();
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
    );
  }

  return (
    <div
      className={`shell${!bootReady ? " is-booting" : ""}${shellExpanded ? " is-expanded" : ""}${barGlassOn ? " has-bar-glass" : ""}${ambientFromWindow ? " has-ambient" : ""}${barGlassOn && !ambientFromWindow ? " is-desktop-glass" : ""}`}
      style={ambientCss}
      data-material={material}
      data-chrome-left={chromeLeft.scheme}
      data-chrome-right={chromeRight.scheme}
    >
      {/* 有窗口：吸色条；桌面+模糊开：仅透出下层 Win32 材质 */}
      {ambientFromWindow ? (
        <AmbientStrip style={stripStyle} />
      ) : null}
      {barGlassOn ? <div className="bar-glass" aria-hidden /> : null}

      {expanded && (
        <div
          className="dismiss-backdrop"
          aria-hidden
          onPointerDown={(e) => {
            // 点岛左右空白：关掉弹窗（透明区原先会被系统点透）
            e.preventDefault();
            trayOpenRef.current = false;
            setTrayOpen(false);
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

      {TRAY_UI_ENABLED ? (
        <TrayCluster open={trayOpen} onOpenChange={setTrayOpen} />
      ) : (
        <ChromeStatusCluster />
      )}

      <BorderBeam
        ref={islandRef}
        size="pulse-inner"
        colorVariant="colorful"
        strength={0.7}
        borderRadius={Math.round(islandBottomRadius(size.width, size.height))}
        active={USE_NOTIFY_BORDER_BEAM && notifyInline}
        className="island-beam"
        style={
          {
            overflow: "visible",
            left: "50%",
            right: "auto",
            translate: "-50% 0",
            width: size.width,
            height: size.height,
            ["--island-r-bot"]: `${islandBottomRadius(size.width, size.height)}px`,
          } as CSSProperties
        }
      >
        <div
          className={`island-root${expanded ? " is-expanded" : ""}${pulling ? " is-pulling" : ""}${springing ? " is-springing" : ""}${immersed ? " is-immersed" : ""}${notifyInline ? " is-notifying" : ""}${showSearchChrome ? " is-searching" : ""}${dropTarget ? " is-drop-target" : ""}${islandBar || dropTarget ? " has-staging" : ""}${resolveIslandBarAdaptive(barPluginId).enabled ? " has-adaptive-bar" : ""}`}
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          aria-label={
            expanded
              ? "收起灵动岛"
              : canShellPullExpand()
                ? "下拉或点击展开灵动岛"
                : "灵动岛"
          }
          data-chrome={
            dropTarget || showSearchChrome
              ? "dark"
              : immersed
                ? chromeCenter.scheme
                : "dark"
          }
          data-notify-accent={
            notifyInline ? msgBanner?.accentColor || undefined : undefined
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
            if (leaveShrinkTimer.current != null) {
              window.clearTimeout(leaveShrinkTimer.current);
              leaveShrinkTimer.current = null;
            }
            // 悬停时预拉高窗口，按下拖动即可立刻跟手
            if (!canShellPullExpand()) return;
            const pid = resolveShellExpandPluginId();
            if (pid) syncLiveExpandedForPlugin(pid);
            if (!expandedRef.current && !busy.current) void ensureExpandedWindow();
          }}
          onPointerLeave={() => {
            void hideChromeHoverTip();
            if (drag.current?.active || expandedRef.current || busy.current) return;
            if (revealRef.current > 0.01) return;
            // 搜索会话未主动关闭：保持岛/窗口尺寸，不因移出鼠标收回
            if (searchModeRef.current || searchLeavingRef.current) return;
            if (leaveShrinkTimer.current != null) {
              window.clearTimeout(leaveShrinkTimer.current);
            }
            leaveShrinkTimer.current = window.setTimeout(() => {
              leaveShrinkTimer.current = null;
              if (drag.current?.active || expandedRef.current || busy.current) return;
              if (revealRef.current > 0.01) return;
              if (searchModeRef.current || searchLeavingRef.current) return;
              void shrinkIslandWindow();
            }, 320);
          }}
          onClick={() => {
            clickTrace(
              "fe-island",
              `click busy=${busy.current} expanded=${expandedRef.current} swipeMoved=${!!swipe.current?.moved}`,
            );
            // 左滑划掉 / 明显滑动后忽略 click，避免误开应用
            if (swipe.current?.dismissed || swipe.current?.moved) return;
            // 冲突叠层通知不在主岛内，点主岛不处理横幅
            const banner = notifyInline ? msgBannerRef.current : null;
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
          <IslandCornerPatches />
          <div
            ref={shapeLayerRef}
            className="island-shape-layer"
            aria-hidden
          >
            <svg
              ref={svgRef}
              className="island-svg"
            >
              <defs>
                <clipPath id="wh-island-inner-clip" clipPathUnits="userSpaceOnUse">
                  <path ref={pathClipRef} />
                </clipPath>
              </defs>
              <path
                ref={pathRef}
                className="island-path"
                style={{ transition: "fill-opacity 240ms ease" } as CSSProperties}
              />
              <path
                ref={pathStrokeRef}
                className="island-notify-inner-stroke"
                clipPath="url(#wh-island-inner-clip)"
                style={
                  {
                    // 仅内联通知描主岛；冲突叠层时描边给下方胶囊
                    stroke: notifyInline
                      ? msgBanner?.accentColor || "#ff2d55"
                      : "transparent",
                    strokeWidth: notifyInline ? 2 : 0,
                  } as CSSProperties
                }
              />
            </svg>
          </div>

          <div
            ref={islandUiRef}
            className="island-ui"
          >
            <div
              className={`island-bar${notifyInline ? " is-notifying" : ""}${
                searchActive && !searchLeaving ? " is-searching" : ""
              }${searchLeaving ? " is-search-leaving" : ""}`}
            >
              <div className={`bar-weather${weatherBarExiting ? " is-exiting" : ""}`}>
                {barText ? (
                  <div
                    className={`bar-staging${dropTarget ? " is-drop-hint" : ""}`}
                    role="button"
                    tabIndex={0}
                    {...hostTipPointerProps(barTitle)}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (dropTarget || showSearchChrome) return;
                      void hideChromeHoverTip();
                      const pid = barPluginId;
                      if (!pid) return;
                      void emit("island-bar-click", { pluginId: pid }).catch(console.error);
                      const rec = pluginRegistry.get(pid);
                      const hasPanel =
                        Boolean(rec?.manifest.slots?.["island.panel"]) &&
                        (rec?.manifest.capabilities ?? []).includes("island.panel");
                      if (hasPanel) void openPluginSession(pid);
                    }}
                    onKeyDown={(e) => {
                      if (e.key !== "Enter" && e.key !== " ") return;
                      e.preventDefault();
                      e.stopPropagation();
                      if (dropTarget || showSearchChrome) return;
                      void hideChromeHoverTip();
                      const pid = barPluginId;
                      if (!pid) return;
                      void emit("island-bar-click", { pluginId: pid }).catch(console.error);
                      const rec = pluginRegistry.get(pid);
                      const hasPanel =
                        Boolean(rec?.manifest.slots?.["island.panel"]) &&
                        (rec?.manifest.capabilities ?? []).includes("island.panel");
                      if (hasPanel) void openPluginSession(pid);
                    }}
                  >
                    {showBarStagingDot ? (
                      <span className="bar-staging-dot" aria-hidden />
                    ) : null}
                    <span className="bar-staging-text" ref={barStagingTextRef} />
                  </div>
                ) : null}
              </div>
              {showSearchChrome ? (
                  <div
                  className={`bar-search${searchLeaving ? " is-exiting" : ""}`}
                  key="island-search"
                >
                  <svg
                    className="bar-search-icon"
                    viewBox="0 0 24 24"
                    fill="none"
                    aria-hidden
                  >
                    <circle
                      cx="10.5"
                      cy="10.5"
                      r="6.25"
                      stroke="currentColor"
                      strokeWidth="1.7"
                    />
                    <path
                      d="M15.2 15.2L20 20"
                      stroke="currentColor"
                      strokeWidth="1.7"
                      strokeLinecap="round"
                    />
                  </svg>
                  <input
                    ref={searchInputRef}
                    className="bar-search-input"
                    type="search"
                    enterKeyHint="search"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="全局搜索，一搜全有"
                    value={searchDraft}
                    onChange={(e) => {
                      const v = e.target.value;
                      setSearchDraft(v);
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === "Enter") {
                        e.preventDefault();
                        submitIslandSearch();
                      } else if (e.key === "Escape") {
                        e.preventDefault();
                        if (expandedRef.current) {
                          retainSearchModeRef.current = false;
                          void collapse();
                        } else void toggleIslandSearchMode();
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="bar-search-btn"
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      submitIslandSearch();
                    }}
                  >
                    搜索
                  </button>
                </div>
              ) : null}
              {notifyInline ? renderNotifyBanner({ stacked: false }) : null}
            </div>

            <div
              ref={panelRef}
              className={`island-panel is-plugin${pluginStagingShell ? " is-plugin-sized" : ""}${
                expanded || reveal > 0.12 ? " is-open" : ""
              }`}
              onClick={(e) => e.stopPropagation()}
            >
              {effectivePullContent || panelOverride ? (
                <IslandPanelHost
                  key={panelHostPluginId}
                  pullContent={effectivePullContent || panelOverride || ""}
                  active={panelActive || expanded}
                  searchSubmit={searchSubmit}
                  onPanelClose={() => {
                    if (expandedRef.current) void collapse();
                  }}
                />
              ) : (
                <div className="panel-plugin-empty">
                  面板未绑定插件
                  <span>请在设置 → 灵动岛中选择「下拉内容」</span>
                </div>
              )}
            </div>
          </div>
        </div>
      </BorderBeam>
      {notifyStacked ? (
        <div
          className="island-notify-stack"
          style={
            {
              top: size.height + NOTIFY_STACK_GAP,
              ["--notify-stack-max-w" as string]: `${Math.max(160, Math.min(size.width, 420))}px`,
            } as CSSProperties
          }
        >
          {renderNotifyBanner({ stacked: true })}
        </div>
      ) : null}
    </div>
  );
}

export default App;
