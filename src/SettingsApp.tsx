import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getIslandPrefs,
  mergeBarPriority,
  hydrateIslandPrefs,
  setIslandPrefs,
  type IslandPrefs,
} from "./islandPrefs";
import { listPanelProviders } from "./plugins/panelProviders";
import { listBarResidentProviders } from "./plugins/islandSlots";
import { pluginRegistry } from "./plugins/registry";
import {
  bootstrapPlugins,
  subscribeInstalledPlugins,
  type InstalledPluginDto,
} from "./plugins/bootstrap";
import { describeCapabilities, isSensitiveCapability } from "./plugins/capGate";
import type { PluginCapability, PluginSettingField } from "./plugins/types";
import { subscribeSystemDark, syncGlassCss } from "./glassPrefs";
import SqliteDevPanel from "./components/SqliteDevPanel";
import PluginSettingsForm from "./components/PluginSettingsForm";
import {
  DEFAULT_SYSTEM_CHIPS,
  isTrayPinned,
  mergeTrayIcons,
  normalizeSystemChips,
  type SystemChipVisibility,
  type TrayIconInfo as SharedTrayIconInfo,
  type TrayPrefs as SharedTrayPrefs,
} from "./components/TrayCluster";

type AmbientMode = "edge" | "center";
type DarkPref = "auto" | "dark" | "light";
type NavId = "general" | "theme" | "dock" | "sousou" | "tray" | "plugins" | "developer";

type DockDisplayMode =
  | "default"
  | "layered"
  | "autoHide"
  | "smartHide"
  | "always"
  | "hotkey"
  | "alwaysFullscreen"
  | "desktop";

type DockItemLite = {
  id: string;
  kind: string;
  label: string;
  matchExe?: string;
  launchPath?: string;
  realPath?: string;
  virtualPath?: string;
  iconPath?: string;
  uwp?: boolean;
  launchArgs?: string;
  appId?: string;
  iconPng?: string | null;
};

type DockPrefs = {
  enabled: boolean;
  displayMode: DockDisplayMode | string;
  hideSystemTaskbar: boolean;
  items: DockItemLite[];
  hotkey: string;
  /** screenBottom (default) | dockBottom */
  activationPosition: "screenBottom" | "dockBottom" | string;
  activationThicknessPx: number;
  bottomOffsetPx: number;
  /** After pointer leaves, wait this many ms before hiding (auto/smart hide). */
  hideLingerMs: number;
  /** Max icon scale on hover (1 = off, up to 2.5). */
  magnification: number;
  /** Hover window thumbnails above running apps. */
  showPreview: boolean;
  /** Floating name label on hover. */
  showHoverLabel: boolean;
  /** Chrome corner radius (Apple pill). */
  cornerRadius: number;
  /** Bounce icon after click. */
  bounceOnClick: boolean;
  /** Icon slot size (28–56). */
  iconSize: number;
  /** Gap between icons (4–24). */
  iconGap: number;
  /** Apple-style peek strip while auto-hidden. */
  showTriggerStrip: boolean;
  /** Show unpinned running apps after a separator. */
  showRunningApps: boolean;
  /** Running indicator: bar | dot */
  indicatorStyle: "bar" | "dot" | string;
  /** Hot corner: bottom-right → show desktop. */
  cornerShowDesktop: boolean;
  /** Hot corner: bottom-left → Start menu. */
  cornerOpenStart: boolean;
};

const DOCK_ACTIVATION_POSITIONS: {
  id: "screenBottom" | "dockBottom";
  label: string;
  desc: string;
}[] = [
  { id: "screenBottom", label: "屏幕最底部", desc: "整条底边热区（默认）" },
  { id: "dockBottom", label: "仅 Dock 宽度", desc: "只在底栏水平范围内触发" },
];

const DOCK_MODES: { id: DockDisplayMode; label: string; desc: string }[] = [
  { id: "default", label: "默认显示模式", desc: "常驻贴底；全屏游戏时隐藏" },
  { id: "layered", label: "叠层显示模式", desc: "常驻并保持置顶" },
  { id: "autoHide", label: "自动隐藏模式", desc: "鼠标靠近激活区时显示" },
  { id: "smartHide", label: "智能隐藏模式", desc: "窗口与 Dock 重叠时隐藏" },
  { id: "always", label: "始终显示模式", desc: "始终显示（普通窗口之上）" },
  { id: "hotkey", label: "热键显示模式", desc: "Ctrl+Alt+D 切换显隐" },
  { id: "alwaysFullscreen", label: "始终显示包括全屏", desc: "尽量在全屏时也保持显示" },
  { id: "desktop", label: "桌面显示模式", desc: "仅在桌面前景时显示" },
];

function normalizeDockPrefs(dp: Partial<DockPrefs> | null | undefined): DockPrefs {
  return {
    enabled: !!dp?.enabled,
    displayMode: dp?.displayMode || "default",
    hideSystemTaskbar: dp?.hideSystemTaskbar !== false,
    items: dp?.items ?? [],
    hotkey: dp?.hotkey || "Ctrl+Alt+D",
    activationPosition:
      dp?.activationPosition === "dockBottom" ? "dockBottom" : "screenBottom",
    activationThicknessPx: Math.min(64, Math.max(4, Number(dp?.activationThicknessPx) || 20)),
    bottomOffsetPx: (() => {
      const n = Number(dp?.bottomOffsetPx);
      // Undo accidental screen-lift defaults from earlier builds.
      if (n === 28 || n === 36) return 4;
      return Math.min(400, Math.max(0, Number.isFinite(n) ? n : 4));
    })(),
    hideLingerMs: Math.min(10000, Math.max(200, Number(dp?.hideLingerMs) || 800)),
    magnification: Math.min(
      2.5,
      Math.max(1, Number.isFinite(Number(dp?.magnification)) ? Number(dp?.magnification) : 1.6),
    ),
    showPreview: dp?.showPreview !== false,
    showHoverLabel: dp?.showHoverLabel === true,
    cornerRadius: Math.min(
      28,
      Math.max(8, Number.isFinite(Number(dp?.cornerRadius)) ? Number(dp?.cornerRadius) : 16),
    ),
    bounceOnClick: dp?.bounceOnClick !== false,
    iconSize: Math.min(
      56,
      Math.max(28, Number.isFinite(Number(dp?.iconSize)) ? Number(dp?.iconSize) : 40),
    ),
    iconGap: Math.min(
      24,
      Math.max(4, Number.isFinite(Number(dp?.iconGap)) ? Number(dp?.iconGap) : 10),
    ),
    showTriggerStrip: dp?.showTriggerStrip !== false,
    showRunningApps: dp?.showRunningApps !== false,
    indicatorStyle: dp?.indicatorStyle === "dot" ? "dot" : "bar",
    cornerShowDesktop: dp?.cornerShowDesktop !== false,
    cornerOpenStart: dp?.cornerOpenStart !== false,
  };
}

type MaterialPrefs = {
  kind: string;
  dark: boolean | null;
  acrylicAlpha: number;
};

type Ambient = {
  r: number;
  g: number;
  b: number;
  png_base64?: string;
};

type TrayIconInfo = SharedTrayIconInfo;

type TrayPrefs = SharedTrayPrefs;

const SYSTEM_CHIP_TOGGLES: { key: keyof SystemChipVisibility; label: string; desc: string }[] = [
  { key: "perf", label: "性能温度", desc: "CPU / GPU 温度与内存占用" },
  { key: "network", label: "网速", desc: "上行 / 下行、进程网速与断网" },
  { key: "wifi", label: "Wi‑Fi", desc: "网络状态与无线列表" },
  { key: "bluetooth", label: "蓝牙", desc: "蓝牙开关与已配对设备" },
  { key: "volume", label: "声音", desc: "音量与输出设备" },
  { key: "power", label: "电源", desc: "电池与电源计划" },
  { key: "peripherals", label: "外设", desc: "耳机 / 手柄等快捷芯片" },
  { key: "ime", label: "输入法", desc: "当前输入法与大小写" },
  { key: "clock", label: "时钟", desc: "日期时间与日历" },
];

type PluginMarketEntry = {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  official: boolean;
  dev: boolean;
  capabilities: PluginCapability[];
  settings?: PluginSettingField[];
  settingsIntro?: string;
};

type PluginSurfacePreview = {
  id: string;
  label: string;
  detail: string;
};

type PluginPreviewDto = {
  id: string;
  name: string;
  version: string;
  capabilities: string[];
  surfaces: PluginSurfacePreview[];
  networkHosts: string[];
  description?: string | null;
};

type InstallPending =
  | { kind: "path"; path: string; preview: PluginPreviewDto }
  | { kind: "example"; exampleId: string; preview: PluginPreviewDto };

type ScriptEnv = "python" | "node" | "powershell" | "cmd" | "exe" | "custom";

type ScriptLauncherRow = {
  id: string;
  name: string;
  scriptPath: string;
  environment: ScriptEnv | string;
  envPath: string;
  args: string;
  pluginId: string;
  startWithHub: boolean;
  startOnBoot: boolean;
  enabled: boolean;
  running: boolean;
  pid?: number | null;
};

const SCRIPT_ENVS: { id: ScriptEnv; label: string }[] = [
  { id: "python", label: "Python" },
  { id: "node", label: "Node.js" },
  { id: "powershell", label: "PowerShell" },
  { id: "cmd", label: "CMD / Bat" },
  { id: "exe", label: "可执行文件" },
  { id: "custom", label: "自定义运行时" },
];

const emptyLauncherDraft = (): Omit<ScriptLauncherRow, "running" | "pid"> => ({
  id: "",
  name: "",
  scriptPath: "",
  environment: "python",
  envPath: "",
  args: "",
  pluginId: "",
  startWithHub: true,
  startOnBoot: false,
  enabled: true,
});

function formatSousouCacheBytes(n: number) {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

function trayLabel(icon: TrayIconInfo) {
  return icon.tooltip || icon.process || "未知应用";
}

/** 与 Rust `DEFAULT_TENCENT_MENU_HEIGHT` 一致 */
const DEFAULT_TENCENT_MENU_HEIGHT = 200;

function isTencentIm(icon: TrayIconInfo) {
  const p = (icon.process || "").trim().toLowerCase();
  if (
    p === "weixin" ||
    p === "wechat" ||
    p === "wechatappex" ||
    p === "qq" ||
    p === "qqnt" ||
    p === "tim" ||
    p.startsWith("weixin") ||
    p.startsWith("wechat") ||
    p.startsWith("qqnt")
  ) {
    return true;
  }
  const t = (icon.tooltip || "").trim();
  return t === "微信" || t === "QQ" || t.startsWith("微信");
}

function menuHeightLabel(icon: TrayIconInfo, heights: Record<string, number>) {
  const custom = heights[icon.id];
  if (custom != null && custom > 0) return `菜单 ${custom}px`;
  if (isTencentIm(icon)) return `菜单 ${DEFAULT_TENCENT_MENU_HEIGHT}px（默认）`;
  return "菜单自动";
}

const NAV: { id: NavId; label: string; tint: string; icon: ReactNode }[] = [
  {
    id: "general",
    label: "全局设置",
    tint: "#0a84ff",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="12" cy="12" r="3" />
        <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
      </svg>
    ),
  },
  {
    id: "theme",
    label: "主题",
    tint: "#bf5af2",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 3v18M3 12h18" />
      </svg>
    ),
  },
  {
    id: "dock",
    label: "Dock",
    tint: "#64d2ff",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="14" width="18" height="6" rx="2" />
        <path d="M7 17h.01M12 17h.01M17 17h.01" />
      </svg>
    ),
  },
  {
    id: "sousou",
    label: "搜搜",
    tint: "#3ecf8e",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="11" cy="11" r="7" />
        <path d="M20 20l-3.5-3.5" />
      </svg>
    ),
  },
  {
    id: "tray",
    label: "托盘",
    tint: "#30d158",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="4" width="18" height="6" rx="2" />
        <path d="M7 14h.01M12 14h.01M17 14h.01M7 18h10" />
      </svg>
    ),
  },
  {
    id: "plugins",
    label: "插件市场",
    tint: "#ff9f0a",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M12 2v6M12 16v6M2 12h6M16 12h6" />
        <circle cx="12" cy="12" r="3" />
      </svg>
    ),
  },
  {
    id: "developer",
    label: "开发者选项",
    tint: "#8e8e93",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M16 18l6-6-6-6M8 6l-6 6 6 6" />
      </svg>
    ),
  },
];

const AMBIENT_MODES: { id: AmbientMode; label: string; desc: string }[] = [
  { id: "edge", label: "整条边缘", desc: "左右按窗口顶边色带贴合" },
  { id: "center", label: "仅取中间", desc: "只采窗口中部，整条颜色一致" },
];

const MATERIAL_INFO = {
  id: "mica-alt",
  label: "系统磨砂",
  desc: "系统亚克力磨砂透底（可透出壁纸，接近开始菜单观感）",
} as const;

const DARK_OPTS: { id: DarkPref; label: string }[] = [
  { id: "auto", label: "跟随系统" },
  { id: "dark", label: "深色" },
  { id: "light", label: "浅色" },
];

const IDLE_OPTIONS = [
  { sec: 3, label: "3 秒" },
  { sec: 5, label: "5 秒" },
  { sec: 8, label: "8 秒" },
  { sec: 12, label: "12 秒" },
  { sec: 20, label: "20 秒" },
  { sec: 30, label: "30 秒" },
  { sec: 60, label: "1 分钟" },
];

export default function SettingsApp() {
  const [nav, setNav] = useState<NavId>("general");
  const [query, setQuery] = useState("");
  const [ambientMode, setAmbientMode] = useState<AmbientMode>("edge");
  const [ambient, setAmbient] = useState<Ambient>({ r: 42, g: 42, b: 46 });
  const [darkPref, setDarkPref] = useState<DarkPref>("dark");
  const [trays, setTrays] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [pinnedProcesses, setPinnedProcesses] = useState<string[]>([]);
  const [muted, setMuted] = useState<string[]>([]);
  const [mutedProcesses, setMutedProcesses] = useState<string[]>([]);
  const [systemChips, setSystemChips] = useState<SystemChipVisibility>(DEFAULT_SYSTEM_CHIPS);
  const [menuHeights, setMenuHeights] = useState<Record<string, number>>({});
  const [menuHeightEditId, setMenuHeightEditId] = useState<string | null>(null);
  const [menuHeightDraft, setMenuHeightDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [dockPrefs, setDockPrefs] = useState<DockPrefs>(() => normalizeDockPrefs(null));
  const [dockMsg, setDockMsg] = useState("");
  const [dockBusy, setDockBusy] = useState(false);
  const [sousouPrefs, setSousouPrefs] = useState({
    enabled: true,
    hotkeyEnabled: true,
    doubleCtrlMs: 350,
    everythingExe: String.raw`D:\app\Everything\Everything.exe`,
    esExe: String.raw`D:\app\Everything\es.exe`,
  });
  const [sousouMsg, setSousouMsg] = useState("");
  const [sousouBusy, setSousouBusy] = useState(false);
  const [sousouEvStatus, setSousouEvStatus] = useState("");
  const [sousouIconCache, setSousouIconCache] = useState<{
    entries: number;
    bytes: number;
  } | null>(null);
  const [islandPrefs, setIslandPrefsState] = useState<IslandPrefs>(() => getIslandPrefs());
  const [openAtLogin, setOpenAtLogin] = useState(false);
  const [openAtLoginBusy, setOpenAtLoginBusy] = useState(false);
  const [shortcutsVisibleIds, setShortcutsVisibleIds] = useState<string[]>([]);
  const [installed, setInstalled] = useState<InstalledPluginDto[]>([]);
  const [pluginMsg, setPluginMsg] = useState("");
  const [pluginBusy, setPluginBusy] = useState(false);
  const [installPending, setInstallPending] = useState<InstallPending | null>(null);
  const [, bumpRegistry] = useState(0);
  const [launchers, setLaunchers] = useState<ScriptLauncherRow[]>([]);
  const [launcherDraft, setLauncherDraft] = useState(emptyLauncherDraft);
  const [launcherMsg, setLauncherMsg] = useState("");
  const [launcherBusy, setLauncherBusy] = useState(false);
  const [backupMsg, setBackupMsg] = useState("");
  const [backupBusy, setBackupBusy] = useState(false);

  const pullOptions = useMemo(() => {
    const panels = listPanelProviders(pluginRegistry.listPanelManifests());
    return panels.map((p) => ({
      id: p.id,
      label: p.label,
      desc: p.description,
    }));
  }, [installed, bumpRegistry]);

  const barResidentOptions = useMemo(() => {
    return listBarResidentProviders();
  }, [installed, bumpRegistry]);

  const pluginEntries = useMemo<PluginMarketEntry[]>(
    () =>
      installed.map((p) => ({
        id: p.id,
        name: p.name,
        version: p.version,
        enabled: p.enabled,
        official: p.manifest?.official === true,
        dev: p.isDev,
        capabilities: (p.capabilities ?? []) as PluginCapability[],
        settings: p.manifest?.settings,
        settingsIntro: p.manifest?.description,
      })),
    [installed],
  );
  const windowGroupsInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.window-groups" ||
      entry.id === "com.window-hub.window-groups__dev",
  );
  const transferStationInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.transfer-station" ||
      entry.id === "com.window-hub.transfer-station__dev",
  );
  const weatherInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.weather" || entry.id === "com.window-hub.weather__dev",
  );
  const mirrorInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.mirror" || entry.id === "com.window-hub.mirror__dev",
  );
  const idiomsInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.idioms" || entry.id === "com.window-hub.idioms__dev",
  );
  const draftInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.draft" || entry.id === "com.window-hub.draft__dev",
  );
  const todoInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.todo" || entry.id === "com.window-hub.todo__dev",
  );
  const lyricsInstalled = pluginEntries.some(
    (entry) =>
      entry.id === "com.window-hub.lyrics" || entry.id === "com.window-hub.lyrics__dev",
  );

  const shortcutsPluginOptions = useMemo(() => {
    return installed
      .filter(
        (p) =>
          p.enabled &&
          (p.manifest?.slots?.shortcuts != null ||
            (p.capabilities ?? []).includes("shortcuts")),
      )
      .map((p) => ({
        id: p.id,
        name: p.manifest?.slots?.shortcuts?.label ?? p.name,
      }));
  }, [installed]);

  const persistShortcutsVisible = async (ids: string[]) => {
    const nextIds = [...new Set(ids.map((s) => s.trim()).filter(Boolean))];
    setShortcutsVisibleIds(nextIds);
    try {
      const next = await invoke<{
        visiblePluginIds?: string[] | null;
        exclusivePluginId?: string | null;
      }>("set_shortcuts_prefs", {
        prefs: { visiblePluginIds: nextIds, exclusivePluginId: null },
      });
      setShortcutsVisibleIds(next.visiblePluginIds ?? []);
    } catch (err) {
      console.error(err);
    }
  };

  const toggleShortcutsVisible = (pluginId: string) => {
    const id = pluginId.trim();
    if (!id) return;
    const has = shortcutsVisibleIds.includes(id);
    const next = has
      ? shortcutsVisibleIds.filter((x) => x !== id)
      : [...shortcutsVisibleIds, id];
    void persistShortcutsVisible(next);
  };

  const dockItemsPayload = (items: DockItemLite[]) =>
    items.map(({ iconPng: _iconPng, ...rest }) => rest);

  const persistDockPrefs = async (patch: Partial<DockPrefs>) => {
    const next: DockPrefs = { ...dockPrefs, ...patch };
    setDockPrefs(next);
    setDockBusy(true);
    setDockMsg("");
    try {
      const prefs = {
        ...next,
        items: dockItemsPayload(next.items) as DockItemLite[],
      };
      const saved = await invoke<DockPrefs>("set_dock_prefs", { prefs });
      setDockPrefs(normalizeDockPrefs(saved));
    } catch (err) {
      console.error(err);
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const importDockIni = async () => {
    setDockBusy(true);
    setDockMsg("");
    try {
      const path = await invoke<string | null>("pick_dockico_file");
      if (!path) {
        setDockBusy(false);
        return;
      }
      const saved = await invoke<DockPrefs>("import_dockico_ini", { path });
      setDockPrefs(normalizeDockPrefs(saved));
      const n = (saved.items ?? []).filter((i) => i.kind !== "separator").length;
      setDockMsg(`已导入 ${n} 个图标`);
    } catch (err) {
      console.error(err);
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const moveDockItem = (index: number, dir: -1 | 1) => {
    const next = index + dir;
    if (next < 0 || next >= dockPrefs.items.length) return;
    const items = [...dockPrefs.items];
    const tmp = items[index]!;
    items[index] = items[next]!;
    items[next] = tmp;
    void persistDockPrefs({ items });
  };

  const removeDockItem = (id: string) => {
    const items = dockPrefs.items.filter((i) => i.id !== id);
    void persistDockPrefs({ items });
  };

  const addDockApp = async () => {
    setDockBusy(true);
    setDockMsg("");
    try {
      const path = await invoke<string | null>("pick_dock_app_file");
      if (!path) {
        setDockBusy(false);
        return;
      }
      const saved = await invoke<DockPrefs>("dock_add_app", { path, afterId: null });
      setDockPrefs(normalizeDockPrefs(saved));
      setDockMsg("已添加应用");
    } catch (err) {
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const addDockSeparator = async () => {
    setDockBusy(true);
    setDockMsg("");
    try {
      const saved = await invoke<DockPrefs>("dock_add_separator", { afterId: null });
      setDockPrefs(normalizeDockPrefs(saved));
      setDockMsg("已添加分隔线");
    } catch (err) {
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const refreshDockIcons = async () => {
    setDockBusy(true);
    setDockMsg("");
    try {
      const prefs = {
        ...dockPrefs,
        items: dockItemsPayload(dockPrefs.items) as DockItemLite[],
      };
      const saved = await invoke<DockPrefs>("set_dock_prefs", { prefs });
      setDockPrefs(normalizeDockPrefs(saved));
      setDockMsg("已重新提取图标");
    } catch (err) {
      console.error(err);
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const dockItemLabel = (item: DockItemLite) => {
    if (item.kind === "separator") return "分隔线";
    if (item.kind === "startmenu") return item.label || "开始菜单";
    if (item.kind === "trash") return item.label || "回收站";
    return item.label || item.matchExe || item.id;
  };

  useEffect(() => {
    void syncGlassCss({
      kind: "mica-alt",
      dark: darkPref === "auto" ? null : darkPref === "dark",
    });

    void (async () => {
      await hydrateIslandPrefs().then(setIslandPrefsState);

      try {
        const boot = await invoke<boolean>("get_open_at_login");
        setOpenAtLogin(!!boot);
      } catch {
        /* noop */
      }

      try {
        const prefs = await invoke<MaterialPrefs>("get_material_prefs");
        setDarkPref(prefs.dark === true ? "dark" : prefs.dark === false ? "light" : "auto");
        await syncGlassCss({
          kind: "mica-alt",
          dark: prefs.dark,
        });
        await invoke("apply_window_effect", {});
        window.setTimeout(() => {
          void invoke("apply_window_effect", {}).catch(() => undefined);
        }, 120);
        window.setTimeout(() => {
          void invoke("apply_window_effect", {}).catch(() => undefined);
        }, 350);
      } catch {
        try {
          await invoke("apply_window_effect", {});
        } catch {
          /* noop */
        }
      }
      try {
        const mode = (await invoke<string>("get_ambient_mode")) as AmbientMode;
        if (mode === "edge" || mode === "center") setAmbientMode(mode);
        const strip = await invoke<Ambient>("sample_ambient_color");
        setAmbient(strip);
      } catch {
        /* noop */
      }
      try {
        const [list, prefs] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
        ]);
        setTrays(list);
        setPinned(prefs.pinned ?? []);
        setPinnedProcesses(prefs.pinned_processes ?? []);
        setMuted(prefs.muted ?? []);
        setMutedProcesses(prefs.muted_processes ?? []);
        setSystemChips(normalizeSystemChips(prefs.system_chips));
        setMenuHeights(prefs.menu_heights ?? {});
      } catch {
        /* noop */
      }
      try {
        const sp = await invoke<{
          visiblePluginIds?: string[] | null;
          exclusivePluginId?: string | null;
        }>("get_shortcuts_prefs");
        const ids = Array.isArray(sp.visiblePluginIds) ? sp.visiblePluginIds : [];
        if (ids.length) setShortcutsVisibleIds(ids);
        else if (sp.exclusivePluginId?.trim()) setShortcutsVisibleIds([sp.exclusivePluginId.trim()]);
        else setShortcutsVisibleIds([]);
      } catch {
        /* noop */
      }
      try {
        const dp = await invoke<DockPrefs>("get_dock_prefs");
        setDockPrefs(normalizeDockPrefs(dp));
      } catch {
        /* noop */
      }
      try {
        const sp = await invoke<{
          enabled: boolean;
          hotkeyEnabled: boolean;
          doubleCtrlMs: number;
          everythingExe: string;
          esExe: string;
        }>("sousou_get_config");
        setSousouPrefs({
          enabled: sp.enabled,
          hotkeyEnabled: sp.hotkeyEnabled,
          doubleCtrlMs: sp.doubleCtrlMs,
          everythingExe: sp.everythingExe,
          esExe: sp.esExe,
        });
        const st = await invoke<{ running: boolean; message: string }>("sousou_everything_status");
        setSousouEvStatus(st.running ? "Everything 运行中" : st.message);
        const ic = await invoke<{ entries: number; bytes: number }>("sousou_icon_cache_stats");
        setSousouIconCache(ic);
      } catch {
        /* noop */
      }
    })();

    const unsubs: Array<() => void> = [];
    void listen<Ambient>("ambient-color", (ev) => {
      setAmbient(ev.payload);
    }).then((fn) => unsubs.push(fn));
    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setTrays((prev) => mergeTrayIcons(prev, ev.payload ?? []));
    }).then((fn) => unsubs.push(fn));
    void listen<TrayPrefs>("tray-prefs", (ev) => {
      setPinned(ev.payload.pinned ?? []);
      setPinnedProcesses(ev.payload.pinned_processes ?? []);
      setMuted(ev.payload.muted ?? []);
      setMutedProcesses(ev.payload.muted_processes ?? []);
      setSystemChips(normalizeSystemChips(ev.payload.system_chips));
      setMenuHeights(ev.payload.menu_heights ?? {});
    }).then((fn) => unsubs.push(fn));
    void listen<{
      visiblePluginIds?: string[] | null;
      exclusivePluginId?: string | null;
    }>("shortcuts-prefs", (ev) => {
      const ids = Array.isArray(ev.payload?.visiblePluginIds)
        ? ev.payload.visiblePluginIds
        : [];
      if (ids.length) setShortcutsVisibleIds(ids);
      else if (ev.payload?.exclusivePluginId?.trim()) {
        setShortcutsVisibleIds([ev.payload.exclusivePluginId.trim()]);
      } else setShortcutsVisibleIds([]);
    }).then((fn) => unsubs.push(fn));

    void bootstrapPlugins().then((list) => {
      setInstalled(list);
      bumpRegistry((n) => n + 1);
    });
    void subscribeInstalledPlugins((list) => {
      setInstalled(list);
      bumpRegistry((n) => n + 1);
    }).then((fn) => unsubs.push(fn));

    void refreshLaunchers();
    void listen("script-launchers-changed", () => {
      void refreshLaunchers();
    }).then((fn) => unsubs.push(fn));

    // Event-driven tray list — no periodic list_tray_icons.

    return () => {
      unsubs.forEach((fn) => fn());
    };
  }, []);

  // 跟随系统：OS 主题变化时同步 CSS + DWM（解析成明确深/浅，不做第三种）
  useEffect(() => {
    if (darkPref !== "auto") return;
    return subscribeSystemDark(() => {
      void (async () => {
        await syncGlassCss({ kind: "mica-alt", dark: null });
        await invoke("apply_window_effect", {}).catch(() => undefined);
      })();
    });
  }, [darkPref]);

  const pinPrefs = useMemo(
    () => ({ pinned, pinned_processes: pinnedProcesses }),
    [pinned, pinnedProcesses],
  );
  const filteredNav = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return NAV;
    return NAV.filter((n) => n.label.toLowerCase().includes(q));
  }, [query]);

  async function changeAmbientMode(next: AmbientMode) {
    setAmbientMode(next);
    try {
      const strip = await invoke<Ambient>("set_ambient_mode", { mode: next });
      setAmbient(strip);
    } catch {
      /* noop */
    }
  }

  async function persistMaterial(partial: { darkPref?: DarkPref }) {
    const darkMode = partial.darkPref ?? darkPref;
    const prefs: MaterialPrefs = {
      kind: "mica-alt",
      dark: darkMode === "auto" ? null : darkMode === "dark",
      acrylicAlpha: 125,
    };
    try {
      const saved = await invoke<MaterialPrefs>("set_material_prefs", { prefs });
      setDarkPref(saved.dark === true ? "dark" : saved.dark === false ? "light" : "auto");
      await syncGlassCss({
        kind: "mica-alt",
        dark: saved.dark,
      });
      // Re-apply DWM mica after theme flip (resolved dark/light, never null immersive)
      await invoke("apply_window_effect", {}).catch(() => undefined);
    } catch (e) {
      console.error(e);
    }
  }

  async function changeDarkPref(next: DarkPref) {
    setDarkPref(next);
    await persistMaterial({ darkPref: next });
  }

  async function updateIslandPrefs(partial: Partial<IslandPrefs>) {
    const next = await setIslandPrefs(partial);
    setIslandPrefsState(next);
  }

  async function toggleOpenAtLogin() {
    if (openAtLoginBusy) return;
    setOpenAtLoginBusy(true);
    const next = !openAtLogin;
    try {
      const applied = await invoke<boolean>("set_open_at_login", { enabled: next });
      setOpenAtLogin(!!applied);
    } catch (err) {
      console.error(err);
    } finally {
      setOpenAtLoginBusy(false);
    }
  }

  async function persistTrayPrefs(
    nextPinned: string[],
    nextHeights: Record<string, number>,
    nextMuted: string[] = muted,
    nextMutedProcesses: string[] = mutedProcesses,
    nextSystemChips: SystemChipVisibility = systemChips,
    nextPinnedProcesses: string[] = pinnedProcesses,
  ) {
    setSaving(true);
    try {
      const prefs = await invoke<TrayPrefs>("set_tray_prefs", {
        pinned: nextPinned,
        pinnedProcesses: nextPinnedProcesses,
        menuHeights: nextHeights,
        muted: nextMuted,
        mutedProcesses: nextMutedProcesses,
        systemChips: nextSystemChips,
      });
      setPinned(prefs.pinned ?? nextPinned);
      setPinnedProcesses(prefs.pinned_processes ?? nextPinnedProcesses);
      setMenuHeights(prefs.menu_heights ?? nextHeights);
      setMuted(prefs.muted ?? nextMuted);
      setMutedProcesses(prefs.muted_processes ?? nextMutedProcesses);
      setSystemChips(normalizeSystemChips(prefs.system_chips ?? nextSystemChips));
    } catch {
      /* noop */
    } finally {
      setSaving(false);
    }
  }

  async function toggleSystemChip(key: keyof SystemChipVisibility) {
    const next = { ...systemChips, [key]: !systemChips[key] };
    setSystemChips(next);
    await persistTrayPrefs(pinned, menuHeights, muted, mutedProcesses, next);
  }

  async function togglePinned(icon: TrayIconInfo) {
    const on = isTrayPinned(icon, pinPrefs);
    const proc = (icon.process || "").trim().toLowerCase();
    let nextPinned: string[];
    let nextProcs: string[];
    if (on) {
      nextPinned = pinned.filter((id) => {
        if (id === icon.id) return false;
        if (!proc) return true;
        const other = trays.find((t) => t.id === id);
        return !other || (other.process || "").trim().toLowerCase() !== proc;
      });
      nextProcs = pinnedProcesses.filter((p) => p.toLowerCase() !== proc);
    } else {
      nextPinned = pinned.includes(icon.id) ? pinned : [...pinned, icon.id];
      nextProcs =
        proc && !pinnedProcesses.some((p) => p.toLowerCase() === proc)
          ? [...pinnedProcesses, proc]
          : pinnedProcesses;
    }
    setPinned(nextPinned);
    setPinnedProcesses(nextProcs);
    await persistTrayPrefs(nextPinned, menuHeights, muted, mutedProcesses, systemChips, nextProcs);
  }

  function isNotifyMuted(icon: TrayIconInfo) {
    if (muted.includes(icon.id)) return true;
    const proc = (icon.process || "").trim().toLowerCase();
    if (proc && mutedProcesses.some((p) => p.toLowerCase() === proc)) return true;
    const tip = (icon.tooltip || "").toLowerCase();
    return mutedProcesses.some((p) => tip.includes(p.toLowerCase()));
  }

  async function toggleNotifyMute(icon: TrayIconInfo) {
    const on = isNotifyMuted(icon);
    const proc = (icon.process || "").trim().toLowerCase();
    let nextMuted = muted.filter((id) => id !== icon.id);
    let nextProcs = mutedProcesses.filter((p) => p.toLowerCase() !== proc);
    if (!on) {
      nextMuted = [...nextMuted, icon.id];
      if (proc && !nextProcs.some((p) => p.toLowerCase() === proc)) {
        nextProcs = [...nextProcs, proc];
      }
    }
    setMuted(nextMuted);
    setMutedProcesses(nextProcs);
    await persistTrayPrefs(pinned, menuHeights, nextMuted, nextProcs);
  }

  function openMenuHeightEditor(id: string) {
    const icon = trays.find((t) => t.id === id);
    const cur = menuHeights[id];
    setMenuHeightEditId(id);
    if (cur != null && cur > 0) {
      setMenuHeightDraft(String(cur));
    } else if (icon && isTencentIm(icon)) {
      setMenuHeightDraft(String(DEFAULT_TENCENT_MENU_HEIGHT));
    } else {
      setMenuHeightDraft("");
    }
  }

  async function saveIconMenuHeight(id: string) {
    const raw = menuHeightDraft.trim();
    const parsed = raw === "" ? null : Number(raw);
    const nextHeights = { ...menuHeights };
    if (parsed == null || !Number.isFinite(parsed) || parsed <= 0) {
      delete nextHeights[id];
    } else {
      nextHeights[id] = Math.round(Math.min(640, Math.max(48, parsed)));
    }
    setMenuHeights(nextHeights);
    setMenuHeightEditId(null);
    setMenuHeightDraft("");
    await persistTrayPrefs(pinned, nextHeights);
  }

  async function clearIconMenuHeight(id: string) {
    const nextHeights = { ...menuHeights };
    delete nextHeights[id];
    setMenuHeights(nextHeights);
    setMenuHeightEditId(null);
    setMenuHeightDraft("");
    await persistTrayPrefs(pinned, nextHeights);
  }

  function toggleInstalled(id: string, enabled: boolean) {
    void invoke("set_plugin_enabled", { id, enabled: !enabled })
      .then(() => bumpRegistry((n) => n + 1))
      .catch((err) => setPluginMsg(String(err)));
  }

  async function refreshLaunchers() {
    try {
      const list = await invoke<ScriptLauncherRow[]>("list_script_launchers");
      setLaunchers(list);
    } catch {
      /* noop */
    }
  }

  function editLauncher(row: ScriptLauncherRow) {
    setLauncherDraft({
      id: row.id,
      name: row.name,
      scriptPath: row.scriptPath,
      environment: row.environment,
      envPath: row.envPath ?? "",
      args: row.args ?? "",
      pluginId: row.pluginId ?? "",
      startWithHub: row.startWithHub,
      startOnBoot: row.startOnBoot,
      enabled: row.enabled,
    });
    setLauncherMsg("");
  }

  async function pickLauncherScript() {
    try {
      const path = await invoke<string | null>("pick_script_file");
      if (!path) return;
      const stem = path.replace(/^.*[\\/]/, "").replace(/\.[^.]+$/, "");
      setLauncherDraft((d) => ({
        ...d,
        scriptPath: path,
        name: d.name.trim() ? d.name : stem,
      }));
    } catch (err) {
      setLauncherMsg(String(err));
    }
  }

  async function saveLauncher() {
    if (!launcherDraft.scriptPath.trim()) {
      setLauncherMsg("请先选择脚本路径");
      return;
    }
    setLauncherBusy(true);
    setLauncherMsg("");
    try {
      await invoke("upsert_script_launcher", {
        launcher: {
          id: launcherDraft.id,
          name: launcherDraft.name,
          scriptPath: launcherDraft.scriptPath,
          environment: launcherDraft.environment,
          envPath: launcherDraft.envPath,
          args: launcherDraft.args,
          pluginId: launcherDraft.pluginId,
          startWithHub: launcherDraft.startWithHub,
          startOnBoot: launcherDraft.startOnBoot,
          enabled: launcherDraft.enabled,
        },
      });
      setLauncherDraft(emptyLauncherDraft());
      setLauncherMsg("已保存脚本启动器");
      await refreshLaunchers();
    } catch (err) {
      setLauncherMsg(String(err));
    } finally {
      setLauncherBusy(false);
    }
  }

  async function runLauncher(id: string, start: boolean) {
    setLauncherBusy(true);
    setLauncherMsg("");
    try {
      if (start) {
        await invoke("start_script_launcher", { id });
        setLauncherMsg("已启动");
      } else {
        await invoke("stop_script_launcher", { id });
        setLauncherMsg("已停止");
      }
      await refreshLaunchers();
    } catch (err) {
      setLauncherMsg(String(err));
    } finally {
      setLauncherBusy(false);
    }
  }

  async function removeLauncher(id: string, name: string) {
    if (!window.confirm(`删除启动器「${name}」？`)) return;
    setLauncherBusy(true);
    try {
      await invoke("delete_script_launcher", { id });
      if (launcherDraft.id === id) setLauncherDraft(emptyLauncherDraft());
      await refreshLaunchers();
    } catch (err) {
      setLauncherMsg(String(err));
    } finally {
      setLauncherBusy(false);
    }
  }

  function deleteInstalled(id: string, name: string) {
    if (!window.confirm(`删除插件「${name}」？`)) return;
    void invoke("uninstall_plugin", { id })
      .then(() => {
        setPluginMsg(`已删除 ${name}`);
        bumpRegistry((n) => n + 1);
      })
      .catch((err) => setPluginMsg(String(err)));
  }

  async function beginInstallFromPath(path: string) {
    setPluginBusy(true);
    setPluginMsg("");
    try {
      const preview = await invoke<PluginPreviewDto>("preview_plugin_from_path", { path });
      setInstallPending({ kind: "path", path, preview });
    } catch (err) {
      setPluginMsg(String(err));
    } finally {
      setPluginBusy(false);
    }
  }

  async function beginInstallExample(exampleId: string) {
    setPluginBusy(true);
    setPluginMsg("");
    try {
      const preview = await invoke<PluginPreviewDto>("preview_example_plugin", {
        exampleId,
      });
      setInstallPending({ kind: "example", exampleId, preview });
    } catch (err) {
      setPluginMsg(String(err));
    } finally {
      setPluginBusy(false);
    }
  }

  async function confirmInstallPending() {
    if (!installPending) return;
    setPluginBusy(true);
    setPluginMsg("");
    try {
      const rec =
        installPending.kind === "path"
          ? await invoke<InstalledPluginDto>("install_plugin_from_path", {
              path: installPending.path,
            })
          : await invoke<InstalledPluginDto>("install_example_plugin", {
              exampleId: installPending.exampleId,
            });
      const caps = (rec.capabilities ?? []) as PluginCapability[];
      const sensitive = caps.filter((c) => isSensitiveCapability(c));
      const verb = installPending.kind === "example" ? "已导入示例" : "已安装";
      setPluginMsg(
        sensitive.length
          ? `${verb}「${rec.name}」。注意：${describeCapabilities(sensitive).join("；")}`
          : `${verb}「${rec.name}」v${rec.version}`,
      );
      setInstallPending(null);
      bumpRegistry((n) => n + 1);
    } catch (err) {
      setPluginMsg(String(err));
    } finally {
      setPluginBusy(false);
    }
  }

  const title = NAV.find((n) => n.id === nav)?.label ?? "设置";

  return (
    <div className="settings-shell">
      <aside className="settings-side">
        <label className="settings-search">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
            <circle cx="11" cy="11" r="7" />
            <path d="M20 20l-3-3" />
          </svg>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索"
            spellCheck={false}
          />
        </label>

        <nav className="settings-nav">
          {filteredNav.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`settings-nav-item${nav === item.id ? " is-active" : ""}`}
              onClick={() => setNav(item.id)}
            >
              <span className="settings-nav-icon" style={{ background: item.tint }}>
                {item.icon}
              </span>
              <span>{item.label}</span>
            </button>
          ))}
        </nav>
      </aside>

      <main className="settings-main">
        <header className="settings-main-head">
          <h1>{title}</h1>
        </header>

        <div className="settings-main-body">
          {nav === "general" && (
            <>
              <section className="settings-card">
                <h2>启动</h2>
                <p className="card-desc">
                  登录 Windows 后自动启动 Window Hub（写入当前用户的「启动」文件夹）。
                </p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">开机自启</span>
                    <span className="pref-row-desc">
                      {openAtLogin ? "已启用：登录后自动运行" : "关闭时需手动打开"}
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${openAtLogin ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={openAtLogin}
                    disabled={openAtLoginBusy}
                    onClick={() => void toggleOpenAtLogin()}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
              </section>
              <section className="settings-card">
                <h2>快捷区</h2>
                <p className="card-desc">
                  状态菜单左侧快捷区可显示多个插件入口。可全开，或勾选若干 shortcuts 插件（例如只保留窗口组与中转站）。
                </p>
                <div className="pref-row pref-row-stack">
                  <span className="pref-row-text">
                    <span className="pref-row-label">快捷区占用</span>
                    <span className="pref-row-desc">
                      未勾选任何项 = 全部插件；勾选后仅显示所选插件
                    </span>
                  </span>
                  <div className="mode-list is-compact">
                    <button
                      type="button"
                      className={`mode-item${shortcutsVisibleIds.length === 0 ? " is-selected" : ""}`}
                      onClick={() => void persistShortcutsVisible([])}
                    >
                      <span className="mode-label">全部插件</span>
                      <span className="mode-desc">显示所有已启用的 shortcuts 入口</span>
                    </button>
                    {shortcutsPluginOptions.map((p) => {
                      const selected = shortcutsVisibleIds.includes(p.id);
                      return (
                        <button
                          key={p.id}
                          type="button"
                          className={`mode-item${selected ? " is-selected" : ""}`}
                          onClick={() => toggleShortcutsVisible(p.id)}
                        >
                          <span className="mode-label">{p.name}</span>
                          <span className="mode-desc">{selected ? "已选入快捷区" : "点击加入快捷区"}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              </section>
              <section className="settings-card">
                <h2>下拉内容</h2>
                <p className="card-desc">
                  选择点击或下拉展开灵动岛时默认显示的内容。列表来自已启用且声明 island.panel、未设
                  excludeFromPullContent 的插件（如天气、镜子）。中转站在左侧快捷区打开弹窗使用。
                </p>
                <div className="mode-list">
                  {pullOptions.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`mode-item${islandPrefs.pullContent === item.id ? " is-selected" : ""}`}
                      onClick={() => updateIslandPrefs({ pullContent: item.id })}
                    >
                      <span className="mode-label">{item.label}</span>
                      <span className="mode-desc">{item.desc}</span>
                    </button>
                  ))}
                </div>
              </section>
              <section className="settings-card">
                <h2>岛栏顺序</h2>
                <p className="card-desc">
                  折叠态按下方顺序竞选第一条有内容的插件摘要（默认歌词 → 待办 → 天气）。通知横幅始终最优先；中转站有条目时临时盖住内容层。
                  使用 ↑ ↓ 调整优先级（越靠上越优先）。
                </p>
                <div className="bar-priority-fixed">
                  <div className="bar-priority-row is-fixed">
                    <span className="bar-priority-rank">1</span>
                    <div className="bar-priority-meta">
                      <span className="mode-label">通知</span>
                      <span className="mode-desc">插件 notify / 托盘闪动（固定最上，不可调）</span>
                    </div>
                  </div>
                </div>
                <div className="mode-list bar-priority-list">
                  {(() => {
                    const order = mergeBarPriority(
                      islandPrefs.barPriority,
                      barResidentOptions.map((p) => p.id),
                      islandPrefs.barResident,
                    );
                    const labelOf = (id: string) =>
                      barResidentOptions.find((p) => p.id === id)?.label ?? id;
                    const descOf = (id: string) =>
                      barResidentOptions.find((p) => p.id === id)?.description ?? "岛栏摘要";
                    const move = (id: string, dir: -1 | 1) => {
                      const next = [...order];
                      const i = next.indexOf(id);
                      const j = i + dir;
                      if (i < 0 || j < 0 || j >= next.length) return;
                      const tmp = next[i]!;
                      next[i] = next[j]!;
                      next[j] = tmp;
                      void updateIslandPrefs({ barPriority: next, barResident: next[0] ?? "" });
                    };
                    if (!order.length) {
                      return (
                        <div className="mode-item" style={{ cursor: "default" }}>
                          <span className="mode-label">暂无岛栏插件</span>
                          <span className="mode-desc">
                            安装并启用声明 island.bar 的插件后会出现在此列表
                          </span>
                        </div>
                      );
                    }
                    return order.map((id, idx) => (
                      <div key={id} className="bar-priority-row">
                        <span className="bar-priority-rank">{idx + 2}</span>
                        <div className="bar-priority-meta">
                          <span className="mode-label">{labelOf(id)}</span>
                          <span className="mode-desc">{descOf(id)}</span>
                        </div>
                        <div className="bar-priority-actions">
                          <button
                            type="button"
                            className="bar-priority-btn"
                            disabled={idx === 0}
                            title="上移"
                            onClick={() => move(id, -1)}
                          >
                            ↑
                          </button>
                          <button
                            type="button"
                            className="bar-priority-btn"
                            disabled={idx >= order.length - 1}
                            title="下移"
                            onClick={() => move(id, 1)}
                          >
                            ↓
                          </button>
                        </div>
                      </div>
                    ));
                  })()}
                </div>
              </section>
              <section className="settings-card">
                <h2>声音</h2>
                <p className="card-desc">调节岛栏音量滑条时，可播放系统提示音以便确认当前音量。</p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">音量调节提示音</span>
                    <span className="pref-row-desc">松手提交音量后播放短提示音</span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${islandPrefs.volumePreviewSound ? " is-on" : ""}`}
                    aria-pressed={islandPrefs.volumePreviewSound}
                    onClick={() =>
                      updateIslandPrefs({ volumePreviewSound: !islandPrefs.volumePreviewSound })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
              </section>
              <section className="settings-card">
                <h2>自动沉浸</h2>
                <p className="card-desc">
                  闲置后岛底变透明，与顶栏背景融为一体；文字颜色会跟设置/时钟一样按背景明暗切换黑白，避免看不见。
                </p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">启用自动沉浸</span>
                    <span className="pref-row-desc">关闭后始终保持黑色岛底</span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${islandPrefs.autoImmerse ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={islandPrefs.autoImmerse}
                    onClick={() => updateIslandPrefs({ autoImmerse: !islandPrefs.autoImmerse })}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${islandPrefs.autoImmerse ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">闲置多久后沉浸</span>
                    <span className="pref-row-desc">期间未点击 / 拖拽岛则自动沉浸</span>
                  </span>
                  <select
                    className="pref-select"
                    value={islandPrefs.immerseIdleSec}
                    disabled={!islandPrefs.autoImmerse}
                    onChange={(e) =>
                      updateIslandPrefs({ immerseIdleSec: Number(e.target.value) })
                    }
                  >
                    {IDLE_OPTIONS.map((opt) => (
                      <option key={opt.sec} value={opt.sec}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                </label>
              </section>
              <section className="settings-card">
                <h2>消息通知</h2>
                <p className="card-desc">
                  微信等应用托盘图标闪动时，退出沉浸并在岛上落下消息提示；点击打开应用或左滑可清掉。应用侧已读、托盘停止闪动后也会自动收起。展示时岛内描一圈绿色内边框。可在「托盘」页对单个应用关闭通知（如 Mem Reduct）。
                </p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">托盘闪动时在岛上提示</span>
                    <span className="pref-row-desc">天气下坠，消息落入居中；点击打开并清除，或左滑划掉</span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${islandPrefs.msgNotify ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={islandPrefs.msgNotify}
                    onClick={() => updateIslandPrefs({ msgNotify: !islandPrefs.msgNotify })}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${islandPrefs.msgNotify ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">默认提示文案</span>
                    <span className="pref-row-desc">无具体通知内容时显示</span>
                  </span>
                  <input
                    className="pref-input"
                    type="text"
                    value={islandPrefs.msgNotifyText}
                    disabled={!islandPrefs.msgNotify}
                    maxLength={24}
                    spellCheck={false}
                    onChange={(e) => updateIslandPrefs({ msgNotifyText: e.target.value })}
                    onBlur={(e) =>
                      updateIslandPrefs({ msgNotifyText: e.target.value.trim() || "收到一条消息" })
                    }
                  />
                </label>
                {(muted.length > 0 || mutedProcesses.length > 0) && (
                  <p className="card-desc">
                    已静音：
                    {[...new Set([
                      ...trays.filter((t) => isNotifyMuted(t)).map((t) => t.tooltip || t.process || t.id),
                      ...mutedProcesses,
                    ])]
                      .filter(Boolean)
                      .slice(0, 8)
                      .join("、") || "若干应用"}
                    。可在「托盘」页重新开启。
                  </p>
                )}
              </section>
              <section className="settings-card">
                <h2>顶栏采样</h2>
                <p className="card-desc">
                  灵动岛顶栏跟随窗口标题栏取色。Electron 应用顶边采黑时会自动采更深或回退壁纸。
                </p>
                <div className="mode-list">
                  {AMBIENT_MODES.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`mode-item${ambientMode === item.id ? " is-selected" : ""}`}
                      onClick={() => void changeAmbientMode(item.id)}
                    >
                      <span className="mode-label">{item.label}</span>
                      <span className="mode-desc">{item.desc}</span>
                    </button>
                  ))}
                </div>
                <label className="pref-row" style={{ marginTop: 12 }}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">顶栏磨砂</span>
                    <span className="pref-row-desc">半透明模糊（接近 MyDockFinder），默认关闭</span>
                  </span>
                  <button
                    type="button"
                    role="switch"
                    className={`pref-switch${islandPrefs.topbarFrost ? " is-on" : ""}`}
                    aria-checked={islandPrefs.topbarFrost}
                    onClick={() => updateIslandPrefs({ topbarFrost: !islandPrefs.topbarFrost })}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
              </section>
              <section className="settings-card">
                <h2>当前顶栏色</h2>
                <div
                  className="ambient-swatch"
                  style={
                    ambientMode === "center" || !ambient.png_base64
                      ? { background: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})` }
                      : {
                          backgroundImage: `url(data:image/png;base64,${ambient.png_base64})`,
                          backgroundSize: "100% 100%",
                        }
                  }
                />
                <p className="swatch-meta">
                  rgb({ambient.r}, {ambient.g}, {ambient.b})
                </p>
              </section>
              <section className="settings-card">
                <h2>数据备份</h2>
                <p className="card-desc">
                  设置、托盘/岛栏偏好、插件配置与数据、已安装插件包均保存在本机
                  <code> %APPDATA%/window-hub </code>
                  。可导出为 <code>.whbak</code> 备份包，换机或重装后导入即可恢复。
                </p>
                <div className="plugin-actions">
                  <button
                    type="button"
                    className="settings-primary-btn"
                    disabled={backupBusy}
                    onClick={() => {
                      void (async () => {
                        setBackupBusy(true);
                        setBackupMsg("");
                        try {
                          const path = await invoke<string | null>("export_hub_backup");
                          setBackupMsg(path ? `已导出：${path}` : "已取消导出");
                        } catch (err) {
                          setBackupMsg(`导出失败：${String(err)}`);
                        } finally {
                          setBackupBusy(false);
                        }
                      })();
                    }}
                  >
                    导出设置与插件
                  </button>
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={backupBusy}
                    onClick={() => {
                      void (async () => {
                        setBackupBusy(true);
                        setBackupMsg("");
                        try {
                          const path = await invoke<string | null>("pick_hub_backup_file");
                          if (!path) {
                            setBackupMsg("已取消导入");
                            return;
                          }
                          const ok = window.confirm(
                            `将用备份覆盖当前设置与已安装插件：\n${path}\n\n建议先导出一份当前备份。导入后若界面异常请重启应用。继续？`,
                          );
                          if (!ok) {
                            setBackupMsg("已取消导入");
                            return;
                          }
                          await invoke("import_hub_backup", { path });
                          setBackupMsg("导入成功。若部分界面仍显示旧数据，请重启应用。");
                        } catch (err) {
                          setBackupMsg(`导入失败：${String(err)}`);
                        } finally {
                          setBackupBusy(false);
                        }
                      })();
                    }}
                  >
                    导入备份
                  </button>
                </div>
                {backupMsg ? <p className="plugin-msg">{backupMsg}</p> : null}
              </section>
            </>
          )}

          {nav === "theme" && (
            <section className="settings-card">
              <h2>窗口材质</h2>
              <p className="card-desc">
                设置窗、托盘弹窗、插件弹窗、Dock 统一使用系统磨砂透底（可透出壁纸）。其他
                DWMBlurGlass 材质暂未开放。
              </p>
              <div className="mode-list">
                <button
                  key={MATERIAL_INFO.id}
                  type="button"
                  className="mode-item is-selected"
                  disabled
                >
                  <span className="mode-label">{MATERIAL_INFO.label}</span>
                  <span className="mode-desc">{MATERIAL_INFO.desc}</span>
                </button>
              </div>

              <div className="material-params">
                <p className="card-desc" style={{ marginTop: 14, marginBottom: 8 }}>
                  磨砂深浅色。「跟随系统」会按 Windows 应用主题解析成深色或浅色（与点选深色/浅色同一套），不会出现第三种混搭。
                </p>
                <div className="mode-list is-compact">
                  {DARK_OPTS.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`mode-item${darkPref === item.id ? " is-selected" : ""}`}
                      onClick={() => void changeDarkPref(item.id)}
                    >
                      <span className="mode-label">{item.label}</span>
                    </button>
                  ))}
                </div>
              </div>
            </section>
          )}

          {nav === "dock" && (
            <>
              <section className="settings-card">
                <h2>底部 Dock</h2>
                <p className="card-desc">
                  Host 自带底栏（非插件）。风格对齐 macOS Dock：圆角毛玻璃、悬停放大/预览、运行中小白条、未固定窗口分隔显示。
                </p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">启用 Dock</span>
                    <span className="pref-row-desc">关闭后隐藏 Dock 并恢复系统任务栏</span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.enabled ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.enabled}
                    disabled={dockBusy}
                    onClick={() => void persistDockPrefs({ enabled: !dockPrefs.enabled })}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">显示模式</span>
                    <span className="pref-row-desc">对齐 MyDockFinder 的八种底栏策略</span>
                  </span>
                  <select
                    className="pref-select"
                    value={dockPrefs.displayMode}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) =>
                      void persistDockPrefs({ displayMode: e.target.value as DockDisplayMode })
                    }
                  >
                    {DOCK_MODES.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                </label>
                <p className="card-desc" style={{ marginTop: 4 }}>
                  {DOCK_MODES.find((m) => m.id === dockPrefs.displayMode)?.desc ?? ""}
                  {dockPrefs.displayMode === "hotkey" ? `（${dockPrefs.hotkey || "Ctrl+Alt+D"}）` : ""}
                </p>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">隐藏系统任务栏</span>
                    <span className="pref-row-desc">
                      启用 Dock 时强制隐藏；关闭 Dock 后恢复原先任务栏设置
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.enabled ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.enabled}
                    disabled
                    title="启用 Dock 时自动隐藏，不可单独关闭"
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">激活位置</span>
                    <span className="pref-row-desc">
                      自动隐藏等模式：鼠标靠近何处唤出 Dock（默认屏幕最底部）
                    </span>
                  </span>
                  <select
                    className="pref-select"
                    value={
                      dockPrefs.activationPosition === "dockBottom"
                        ? "dockBottom"
                        : "screenBottom"
                    }
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) =>
                      void persistDockPrefs({
                        activationPosition: e.target.value as "screenBottom" | "dockBottom",
                      })
                    }
                  >
                    {DOCK_ACTIVATION_POSITIONS.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.label}
                      </option>
                    ))}
                  </select>
                </label>
                <p className="card-desc" style={{ marginTop: 4 }}>
                  {
                    DOCK_ACTIVATION_POSITIONS.find(
                      (p) => p.id === (dockPrefs.activationPosition === "dockBottom"
                        ? "dockBottom"
                        : "screenBottom"),
                    )?.desc
                  }
                </p>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">激活热区厚度</span>
                    <span className="pref-row-desc">逻辑像素，默认 20</span>
                  </span>
                  <input
                    className="pref-select"
                    type="number"
                    min={4}
                    max={64}
                    step={1}
                    value={dockPrefs.activationThicknessPx}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) => {
                      const n = Number(e.target.value);
                      if (!Number.isFinite(n)) return;
                      setDockPrefs((p) => ({ ...p, activationThicknessPx: n }));
                    }}
                    onBlur={(e) => {
                      const n = Math.min(64, Math.max(4, Number(e.target.value) || 20));
                      void persistDockPrefs({ activationThicknessPx: n });
                    }}
                    style={{ width: 72, textAlign: "right" }}
                  />
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">Dock 底边偏移</span>
                    <span className="pref-row-desc">
                      Dock 底边距屏幕底的间距（贴底默认 4，可设 0）
                    </span>
                  </span>
                  <input
                    className="pref-select"
                    type="number"
                    min={0}
                    max={400}
                    step={1}
                    value={dockPrefs.bottomOffsetPx}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) => {
                      const n = Number(e.target.value);
                      if (!Number.isFinite(n)) return;
                      setDockPrefs((p) => ({ ...p, bottomOffsetPx: n }));
                    }}
                    onBlur={(e) => {
                      const n = Math.min(400, Math.max(0, Number(e.target.value) || 4));
                      void persistDockPrefs({ bottomOffsetPx: n });
                    }}
                    style={{ width: 72, textAlign: "right" }}
                  />
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">右下角 → 显示桌面</span>
                    <span className="pref-row-desc">
                      鼠标移到屏幕最右下角时，切换显示桌面（Win+D）
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.cornerShowDesktop ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.cornerShowDesktop}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() =>
                      void persistDockPrefs({ cornerShowDesktop: !dockPrefs.cornerShowDesktop })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">左下角 → 开始菜单</span>
                    <span className="pref-row-desc">
                      鼠标移到屏幕最左下角时，打开 Windows 开始菜单
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.cornerOpenStart ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.cornerOpenStart}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() =>
                      void persistDockPrefs({ cornerOpenStart: !dockPrefs.cornerOpenStart })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">离开后隐藏延迟</span>
                    <span className="pref-row-desc">
                      自动/智能隐藏：鼠标离开 Dock 后，等待多久再收起（毫秒，默认 800）
                    </span>
                  </span>
                  <input
                    className="pref-select"
                    type="number"
                    min={200}
                    max={10000}
                    step={100}
                    value={dockPrefs.hideLingerMs}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) => {
                      const n = Number(e.target.value);
                      if (!Number.isFinite(n)) return;
                      setDockPrefs((p) => ({ ...p, hideLingerMs: n }));
                    }}
                    onBlur={(e) => {
                      const n = Math.min(10000, Math.max(200, Number(e.target.value) || 800));
                      void persistDockPrefs({ hideLingerMs: n });
                    }}
                    style={{ width: 88, textAlign: "right" }}
                  />
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">显示触发条</span>
                    <span className="pref-row-desc">
                      自动/智能隐藏收起后，在屏幕底边显示一条细白线（类似 macOS Dock）
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.showTriggerStrip ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.showTriggerStrip}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() =>
                      void persistDockPrefs({ showTriggerStrip: !dockPrefs.showTriggerStrip })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
              </section>

              <section className={`settings-card${dockPrefs.enabled ? "" : " is-disabled"}`}>
                <div className="section-head">
                  <h2>外观与交互</h2>
                  <span className="section-hint">放大 · 指示器 · 预览</span>
                </div>
                <p className="card-desc">
                  悬停扇形放大时底栏胶囊会随之增高；运行中的应用用小白条标记，未固定窗口会出现在分隔线右侧。
                </p>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">显示未固定窗口</span>
                    <span className="pref-row-desc">
                      类似 macOS：已打开但未钉在 Dock 上的应用，显示在分隔线右侧
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.showRunningApps ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.showRunningApps}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() =>
                      void persistDockPrefs({ showRunningApps: !dockPrefs.showRunningApps })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">运行指示器</span>
                    <span className="pref-row-desc">图标下方标记正在运行的应用</span>
                  </span>
                  <select
                    className="pref-select"
                    value={dockPrefs.indicatorStyle === "dot" ? "dot" : "bar"}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onChange={(e) =>
                      void persistDockPrefs({
                        indicatorStyle: e.target.value === "dot" ? "dot" : "bar",
                      })
                    }
                  >
                    <option value="bar">小白条（默认）</option>
                    <option value="dot">圆点</option>
                  </select>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">图标大小</span>
                    <span className="pref-row-desc">图标槽位边长（28–56，默认 40）</span>
                  </span>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <input
                      type="range"
                      min={28}
                      max={56}
                      step={1}
                      value={dockPrefs.iconSize}
                      disabled={!dockPrefs.enabled || dockBusy}
                      onChange={(e) => {
                        const n = Number(e.target.value);
                        if (!Number.isFinite(n)) return;
                        setDockPrefs((p) => ({ ...p, iconSize: n }));
                      }}
                      onPointerUp={(e) => {
                        const n = Math.min(
                          56,
                          Math.max(28, Number((e.target as HTMLInputElement).value) || 40),
                        );
                        void persistDockPrefs({ iconSize: n });
                      }}
                      style={{ width: 120 }}
                    />
                    <span style={{ minWidth: 28, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                      {dockPrefs.iconSize}
                    </span>
                  </span>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">图标间距</span>
                    <span className="pref-row-desc">
                      图标之间的间隔（4–24，默认 10）。放大时过小容易重叠
                    </span>
                  </span>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <input
                      type="range"
                      min={4}
                      max={24}
                      step={1}
                      value={dockPrefs.iconGap}
                      disabled={!dockPrefs.enabled || dockBusy}
                      onChange={(e) => {
                        const n = Number(e.target.value);
                        if (!Number.isFinite(n)) return;
                        setDockPrefs((p) => ({ ...p, iconGap: n }));
                      }}
                      onPointerUp={(e) => {
                        const n = Math.min(
                          24,
                          Math.max(4, Number((e.target as HTMLInputElement).value) || 10),
                        );
                        void persistDockPrefs({ iconGap: n });
                      }}
                      style={{ width: 120 }}
                    />
                    <span style={{ minWidth: 28, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                      {dockPrefs.iconGap}
                    </span>
                  </span>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">图标放大</span>
                    <span className="pref-row-desc">
                      划过扇形放大；胶囊高度随放大增长。1.0 = 关闭，默认 1.6
                    </span>
                  </span>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <input
                      type="range"
                      min={1}
                      max={2.5}
                      step={0.05}
                      value={dockPrefs.magnification}
                      disabled={!dockPrefs.enabled || dockBusy}
                      onChange={(e) => {
                        const n = Number(e.target.value);
                        if (!Number.isFinite(n)) return;
                        setDockPrefs((p) => ({ ...p, magnification: n }));
                      }}
                      onPointerUp={(e) => {
                        const n = Math.min(
                          2.5,
                          Math.max(1, Number((e.target as HTMLInputElement).value) || 1.6),
                        );
                        void persistDockPrefs({ magnification: n });
                      }}
                      style={{ width: 120 }}
                    />
                    <span style={{ minWidth: 36, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                      {dockPrefs.magnification.toFixed(2)}
                    </span>
                  </span>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">窗口预览</span>
                    <span className="pref-row-desc">
                      悬停运行中的应用时，在图标上方显示窗口缩略图
                    </span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.showPreview ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.showPreview}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() => void persistDockPrefs({ showPreview: !dockPrefs.showPreview })}
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">点击弹跳</span>
                    <span className="pref-row-desc">点击图标时轻微上跳反馈</span>
                  </span>
                  <button
                    type="button"
                    className={`pref-switch${dockPrefs.bounceOnClick ? " is-on" : ""}`}
                    role="switch"
                    aria-checked={dockPrefs.bounceOnClick}
                    disabled={!dockPrefs.enabled || dockBusy}
                    onClick={() =>
                      void persistDockPrefs({ bounceOnClick: !dockPrefs.bounceOnClick })
                    }
                  >
                    <span className="pref-switch-knob" />
                  </button>
                </label>
                <label className={`pref-row${dockPrefs.enabled ? "" : " is-disabled"}`}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">圆角半径</span>
                    <span className="pref-row-desc">底栏胶囊圆角（默认 16）</span>
                  </span>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <input
                      type="range"
                      min={8}
                      max={28}
                      step={1}
                      value={dockPrefs.cornerRadius}
                      disabled={!dockPrefs.enabled || dockBusy}
                      onChange={(e) => {
                        const n = Number(e.target.value);
                        if (!Number.isFinite(n)) return;
                        setDockPrefs((p) => ({ ...p, cornerRadius: n }));
                      }}
                      onPointerUp={(e) => {
                        const n = Math.min(
                          28,
                          Math.max(8, Number((e.target as HTMLInputElement).value) || 16),
                        );
                        void persistDockPrefs({ cornerRadius: n });
                      }}
                      style={{ width: 120 }}
                    />
                    <span style={{ minWidth: 28, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                      {dockPrefs.cornerRadius}
                    </span>
                  </span>
                </label>
              </section>

              <section className="settings-card">
                <div className="section-head">
                  <h2>图标配置</h2>
                  <span className="section-hint">
                    {dockPrefs.items.filter((i) => i.kind !== "separator").length} 个图标
                    {dockPrefs.items.some((i) => i.kind === "separator")
                      ? ` · ${dockPrefs.items.filter((i) => i.kind === "separator").length} 分隔`
                      : ""}
                  </span>
                </div>
                <p className="card-desc">
                  可直接添加应用 / 分隔线，或从 MyDockFinder 备份导入 `.dockico.ini`。Edge/Chrome
                  「安装的应用」（如 ChatGPT、Gemini）请选桌面或开始菜单里的 `.lnk`；也可先打开应用，在
                  Dock 临时图标上右键「固定到 Dock」。
                </p>
                <div className="plugin-actions">
                  <button
                    type="button"
                    className="settings-primary-btn"
                    disabled={dockBusy}
                    onClick={() => void addDockApp()}
                  >
                    添加应用…
                  </button>
                  <button
                    type="button"
                    className="settings-ghost-btn"
                    disabled={dockBusy}
                    onClick={() => void addDockSeparator()}
                  >
                    添加分隔线
                  </button>
                  <button
                    type="button"
                    className="settings-ghost-btn"
                    disabled={dockBusy}
                    onClick={() => void importDockIni()}
                  >
                    {dockBusy ? "处理中…" : "导入 .dockico.ini"}
                  </button>
                  <button
                    type="button"
                    className="settings-ghost-btn"
                    disabled={dockBusy || dockPrefs.items.length === 0}
                    onClick={() => void refreshDockIcons()}
                  >
                    刷新图标
                  </button>
                </div>
                {dockMsg ? <p className="card-desc">{dockMsg}</p> : null}
                {dockPrefs.items.length === 0 ? (
                  <p className="tray-settings-empty">尚未配置图标，请添加应用或导入 .dockico.ini</p>
                ) : (
                  <div className="dock-settings-list">
                    {dockPrefs.items.map((item, index) => {
                      const label = dockItemLabel(item);
                      const isSep = item.kind === "separator";
                      const missingIcon = !isSep && !item.iconPng;
                      return (
                        <div
                          key={item.id}
                          className={`dock-settings-row${missingIcon ? " is-missing-icon" : ""}`}
                        >
                          <div className="dock-settings-item">
                            {isSep ? (
                              <span className="dock-settings-sep" aria-hidden />
                            ) : item.iconPng ? (
                              <img
                                className="dock-settings-icon"
                                src={`data:image/png;base64,${item.iconPng}`}
                                alt=""
                              />
                            ) : (
                              <span className="dock-settings-icon-fallback" aria-hidden>
                                {(label || "?").charAt(0)}
                              </span>
                            )}
                            <span className="dock-settings-meta">
                              <strong>{label}</strong>
                              <span>
                                {isSep
                                  ? "分隔线"
                                  : missingIcon
                                    ? "图标未解析 · 可刷新或删除"
                                    : item.kind === "startmenu" || item.kind === "trash"
                                      ? item.kind
                                      : item.matchExe || item.launchPath || item.kind}
                              </span>
                            </span>
                          </div>
                          <div className="dock-settings-actions">
                            <button
                              type="button"
                              className="dock-settings-btn"
                              disabled={dockBusy || index === 0}
                              title="上移"
                              onClick={() => moveDockItem(index, -1)}
                            >
                              ↑
                            </button>
                            <button
                              type="button"
                              className="dock-settings-btn"
                              disabled={dockBusy || index >= dockPrefs.items.length - 1}
                              title="下移"
                              onClick={() => moveDockItem(index, 1)}
                            >
                              ↓
                            </button>
                            <button
                              type="button"
                              className="dock-settings-btn is-danger"
                              disabled={dockBusy}
                              title="删除"
                              onClick={() => removeDockItem(item.id)}
                            >
                              删
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </section>
            </>
          )}

          {nav === "sousou" && (
            <section className="settings-card">
              <h2>搜搜</h2>
              <p className="card-desc">
                Host 内置桌面搜索启动器。默认双击 Ctrl 唤起；文件索引依赖 Everything（推荐
                D:\app\Everything）。配置保存在数据库 `prefs_sousou`，会随「开发者选项 →
                备份 / 恢复」一并导出。窗口为 Win10 浅色实底。
              </p>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">启用搜搜</span>
                  <span className="pref-row-desc">关闭后不再响应双击 Ctrl</span>
                </span>
                <button
                  type="button"
                  className={`pref-switch${sousouPrefs.enabled ? " is-on" : ""}`}
                  role="switch"
                  aria-checked={sousouPrefs.enabled}
                  disabled={sousouBusy}
                  onClick={() => {
                    const next = { ...sousouPrefs, enabled: !sousouPrefs.enabled };
                    setSousouPrefs(next);
                    setSousouBusy(true);
                    void invoke("sousou_get_config")
                      .then((full: unknown) =>
                        invoke("sousou_set_config", {
                          prefs: { ...(full as object), ...next },
                        }),
                      )
                      .then(() => setSousouMsg("已保存"))
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                >
                  <span className="pref-switch-knob" />
                </button>
              </label>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">双击 Ctrl 热键</span>
                  <span className="pref-row-desc">连续两次松开 Ctrl 唤起/隐藏</span>
                </span>
                <button
                  type="button"
                  className={`pref-switch${sousouPrefs.hotkeyEnabled ? " is-on" : ""}`}
                  role="switch"
                  aria-checked={sousouPrefs.hotkeyEnabled}
                  disabled={sousouBusy || !sousouPrefs.enabled}
                  onClick={() => {
                    const next = { ...sousouPrefs, hotkeyEnabled: !sousouPrefs.hotkeyEnabled };
                    setSousouPrefs(next);
                    setSousouBusy(true);
                    void invoke("sousou_get_config")
                      .then((full: unknown) =>
                        invoke("sousou_set_config", {
                          prefs: { ...(full as object), ...next },
                        }),
                      )
                      .then(() => setSousouMsg("已保存"))
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                >
                  <span className="pref-switch-knob" />
                </button>
              </label>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">双击间隔 (ms)</span>
                  <span className="pref-row-desc">两次 Ctrl 松开的最大间隔</span>
                </span>
                <input
                  className="pref-select"
                  type="number"
                  min={100}
                  max={2000}
                  value={sousouPrefs.doubleCtrlMs}
                  disabled={sousouBusy}
                  onChange={(e) =>
                    setSousouPrefs((p) => ({
                      ...p,
                      doubleCtrlMs: Number(e.target.value) || 350,
                    }))
                  }
                  onBlur={() => {
                    setSousouBusy(true);
                    void invoke("sousou_get_config")
                      .then((full: unknown) =>
                        invoke("sousou_set_config", {
                          prefs: { ...(full as object), ...sousouPrefs },
                        }),
                      )
                      .then(() => setSousouMsg("已保存"))
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                />
              </label>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">Everything.exe</span>
                </span>
                <input
                  className="pref-select"
                  style={{ minWidth: 280 }}
                  value={sousouPrefs.everythingExe}
                  disabled={sousouBusy}
                  onChange={(e) =>
                    setSousouPrefs((p) => ({ ...p, everythingExe: e.target.value }))
                  }
                  onBlur={() => {
                    setSousouBusy(true);
                    void invoke("sousou_get_config")
                      .then((full: unknown) =>
                        invoke("sousou_set_config", {
                          prefs: { ...(full as object), ...sousouPrefs },
                        }),
                      )
                      .then(() => setSousouMsg("已保存"))
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                />
              </label>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">es.exe</span>
                </span>
                <input
                  className="pref-select"
                  style={{ minWidth: 280 }}
                  value={sousouPrefs.esExe}
                  disabled={sousouBusy}
                  onChange={(e) => setSousouPrefs((p) => ({ ...p, esExe: e.target.value }))}
                  onBlur={() => {
                    setSousouBusy(true);
                    void invoke("sousou_get_config")
                      .then((full: unknown) =>
                        invoke("sousou_set_config", {
                          prefs: { ...(full as object), ...sousouPrefs },
                        }),
                      )
                      .then(() => setSousouMsg("已保存"))
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                />
              </label>
              <p className="card-desc">{sousouEvStatus || "检测中…"}</p>
              <label className="pref-row">
                <span className="pref-row-text">
                  <span className="pref-row-label">图标缓存</span>
                  <span className="pref-row-desc">
                    {sousouIconCache
                      ? `${sousouIconCache.entries} 个 · ${formatSousouCacheBytes(sousouIconCache.bytes)}（磁盘 + 内存）`
                      : "统计加载中…"}
                  </span>
                </span>
                <button
                  type="button"
                  className="settings-ghost-btn"
                  disabled={sousouBusy}
                  onClick={() => {
                    setSousouBusy(true);
                    void invoke<{ entries: number; bytes: number }>("sousou_clear_icon_cache")
                      .then((ic) => {
                        setSousouIconCache(ic);
                        setSousouMsg("已清除图标缓存");
                      })
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                >
                  清除缓存
                </button>
              </label>
              <div className="plugin-actions" style={{ marginTop: 10, marginBottom: 0 }}>
                <button
                  type="button"
                  className="settings-ghost-btn"
                  disabled={sousouBusy}
                  onClick={() => {
                    setSousouBusy(true);
                    void invoke<{ running: boolean; message: string }>("sousou_ensure_everything")
                      .then((st) => {
                        setSousouEvStatus(st.running ? "Everything 运行中" : st.message);
                        setSousouMsg(st.running ? "已启动" : st.message);
                      })
                      .catch((e) => setSousouMsg(String(e)))
                      .finally(() => setSousouBusy(false));
                  }}
                >
                  启动 / 检测 Everything
                </button>
                <button
                  type="button"
                  className="settings-ghost-btn"
                  disabled={sousouBusy}
                  onClick={() => {
                    void invoke("sousou_open").catch((e) => setSousouMsg(String(e)));
                  }}
                >
                  打开搜搜窗口
                </button>
              </div>
              {sousouMsg && <p className="card-desc">{sousouMsg}</p>}
            </section>
          )}

          {nav === "tray" && (
            <>
              <section className="settings-card">
                <div className="section-head">
                  <h2>系统芯片</h2>
                  <span className="section-hint">
                    {saving ? "保存中…" : "控制岛栏右侧 Wi‑Fi / 蓝牙等是否显示"}
                  </span>
                </div>
                <p className="card-desc">
                  关闭后对应芯片从岛栏右侧消失；不影响系统本身的网络 / 蓝牙功能。展开托盘箭头始终保留。
                </p>
                <div>
                  {SYSTEM_CHIP_TOGGLES.map((item) => {
                    const on = systemChips[item.key];
                    return (
                      <label key={item.key} className="pref-row">
                        <span className="pref-row-text">
                          <span className="pref-row-label">{item.label}</span>
                          <span className="pref-row-desc">{item.desc}</span>
                        </span>
                        <button
                          type="button"
                          className={`pref-switch${on ? " is-on" : ""}`}
                          aria-pressed={on}
                          onClick={() => void toggleSystemChip(item.key)}
                        >
                          <span className="pref-switch-knob" />
                        </button>
                      </label>
                    );
                  })}
                </div>
              </section>

              <section className="settings-card settings-card-grow">
              <div className="section-head">
                <h2>托盘常显</h2>
                <span className="section-hint">
                  {saving
                    ? "保存中…"
                    : trays.length > 0
                      ? "勾选常显；铃铛关闭岛通知；齿轮设菜单高度"
                      : "正在抓取系统托盘…"}
                </span>
              </div>
              <p className="card-desc">
                仅列出应用托盘图标。系统芯片（网络 / 音量 / 电源 / 蓝牙 / 输入法 / 时钟）请在上方单独开关。
                右键菜单默认自动测量高度；铃铛关闭后该应用托盘闪动不再弹出岛通知。
              </p>
              {trays.length === 0 ? (
                <p className="tray-settings-empty">暂未收到托盘图标</p>
              ) : (
                <div className="tray-settings-list">
                  {trays.map((icon) => {
                    const on = isTrayPinned(icon, pinPrefs);
                    const notifyMuted = isNotifyMuted(icon);
                    const customH = menuHeights[icon.id];
                    const editing = menuHeightEditId === icon.id;
                    const tencentDefault = isTencentIm(icon);
                    return (
                      <div
                        key={icon.id}
                        className={`tray-settings-row${on ? " is-on" : ""}${editing ? " is-editing" : ""}${notifyMuted ? " is-muted" : ""}`}
                      >
                        <button
                          type="button"
                          className={`tray-settings-item${on ? " is-on" : ""}`}
                          onClick={() => void togglePinned(icon)}
                        >
                          {icon.icon_png_base64 ? (
                            <img
                              className="tray-settings-icon"
                              src={`data:image/png;base64,${icon.icon_png_base64}`}
                              alt=""
                              draggable={false}
                            />
                          ) : (
                            <span className="tray-settings-icon tray-settings-fallback">
                              {trayLabel(icon).charAt(0).toUpperCase()}
                            </span>
                          )}
                          <span className="tray-settings-meta">
                            <span className="tray-settings-name">{trayLabel(icon)}</span>
                            <span className="tray-settings-sub">
                              {icon.process || icon.id}
                              {icon.area === "overflow" ? " · 溢出区" : ""}
                              {notifyMuted ? " · 已静音" : ""}
                              {` · ${menuHeightLabel(icon, menuHeights)}`}
                            </span>
                          </span>
                          <span className={`tray-check${on ? " is-on" : ""}`} aria-hidden>
                            {on ? "✓" : ""}
                          </span>
                        </button>
                        <button
                          type="button"
                          className={`tray-settings-mute${notifyMuted ? " is-on" : ""}`}
                          title={notifyMuted ? "允许岛通知" : "勿打扰（不弹岛通知）"}
                          aria-label={`${trayLabel(icon)} ${notifyMuted ? "允许通知" : "勿打扰"}`}
                          aria-pressed={notifyMuted}
                          onClick={(e) => {
                            e.stopPropagation();
                            void toggleNotifyMute(icon);
                          }}
                        >
                          {notifyMuted ? (
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                              <path d="M13.73 21a2 2 0 0 1-3.46 0" />
                              <path d="M18.63 13A17.89 17.89 0 0 1 18 8" />
                              <path d="M6.26 6.26A5.86 5.86 0 0 0 6 8c0 7-3 9-3 9h14" />
                              <path d="M18 8a6 6 0 0 0-9.33-5" />
                              <line x1="1" y1="1" x2="23" y2="23" />
                            </svg>
                          ) : (
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                              <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
                              <path d="M13.73 21a2 2 0 0 1-3.46 0" />
                            </svg>
                          )}
                        </button>
                        <button
                          type="button"
                          className="tray-settings-gear"
                          title="右键菜单高度"
                          aria-label={`${trayLabel(icon)} 菜单高度`}
                          onClick={(e) => {
                            e.stopPropagation();
                            if (editing) {
                              setMenuHeightEditId(null);
                              setMenuHeightDraft("");
                            } else {
                              openMenuHeightEditor(icon.id);
                            }
                          }}
                        >
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                            <circle cx="12" cy="12" r="3" />
                            <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
                          </svg>
                        </button>
                        {editing ? (
                          <div className="tray-settings-height">
                            <input
                              className="pref-input pref-input-sm"
                              type="number"
                              min={48}
                              max={640}
                              placeholder={tencentDefault ? String(DEFAULT_TENCENT_MENU_HEIGHT) : "自动"}
                              value={menuHeightDraft}
                              autoFocus
                              onChange={(e) => setMenuHeightDraft(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  void saveIconMenuHeight(icon.id);
                                }
                                if (e.key === "Escape") {
                                  setMenuHeightEditId(null);
                                  setMenuHeightDraft("");
                                }
                              }}
                            />
                            <button
                              type="button"
                              className="settings-ghost-btn"
                              onClick={() => void saveIconMenuHeight(icon.id)}
                            >
                              保存
                            </button>
                            <button
                              type="button"
                              className="settings-ghost-btn"
                              disabled={customH == null && !tencentDefault}
                              onClick={() => void clearIconMenuHeight(icon.id)}
                            >
                              {tencentDefault ? "恢复默认" : "自动"}
                            </button>
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
            </>
          )}

          {nav === "plugins" && (
            <section className="settings-card">
              <h2>插件市场</h2>
              <p className="card-desc">
                安装 `.whpx` 包或开发目录（含 plugin.json）。安装前会预览所用界面表面（快捷区/弹窗、岛通知、岛下拉等）与能力声明。也可导入内置示例。
              </p>
              <div className="plugin-actions">
                <button
                  type="button"
                  className="settings-primary-btn"
                  disabled={pluginBusy}
                  onClick={() => {
                    void (async () => {
                      try {
                        const path = await invoke<string | null>("pick_whpx_file");
                        if (!path) return;
                        await beginInstallFromPath(path);
                      } catch (err) {
                        setPluginMsg(String(err));
                      }
                    })();
                  }}
                >
                  安装 .whpx
                </button>
                <button
                  type="button"
                  className="settings-secondary-btn"
                  disabled={pluginBusy}
                  onClick={() => {
                    void (async () => {
                      try {
                        const path = await invoke<string | null>("pick_plugin_directory");
                        if (!path) return;
                        await beginInstallFromPath(path);
                      } catch (err) {
                        setPluginMsg(String(err));
                      }
                    })();
                  }}
                >
                  添加开发目录
                </button>
                {!windowGroupsInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("window-groups")}
                  >
                    导入示例：窗口组
                  </button>
                ) : null}
                {!transferStationInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("transfer-station")}
                  >
                    导入示例：中转站
                  </button>
                ) : null}
                {!weatherInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("weather")}
                  >
                    导入示例：天气
                  </button>
                ) : null}
                {!mirrorInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("mirror")}
                  >
                    导入示例：镜子
                  </button>
                ) : null}
                {!idiomsInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("idioms")}
                  >
                    导入示例：背成语
                  </button>
                ) : null}
                {!draftInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("draft")}
                  >
                    导入示例：随心记
                  </button>
                ) : null}
                {!todoInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("todo")}
                  >
                    导入示例：待办
                  </button>
                ) : null}
                {!lyricsInstalled ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={pluginBusy}
                    onClick={() => void beginInstallExample("lyrics")}
                  >
                    导入示例：歌词
                  </button>
                ) : null}
              </div>
              {!idiomsInstalled || !draftInstalled ? (
                <p className="plugin-msg" style={{ marginTop: 6 }}>
                  「背成语 / 随心记」启用后出现在左侧快捷区；点击芯片打开弹窗。首次启动缺失时会尝试自动安装。
                </p>
              ) : null}
              {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}

              <div className="plugin-list">
                {pluginEntries.map((entry) => (
                  <div
                    key={entry.id}
                    className={`plugin-row${entry.official ? " is-official" : ""}${entry.enabled ? "" : " is-disabled"}`}
                  >
                    <div className="plugin-meta">
                      <strong>
                        {entry.name}
                        {entry.dev ? " (dev)" : ""}
                      </strong>
                      <span>
                        {entry.id} · v{entry.version}
                        {entry.official ? " · 官方" : ""}
                      </span>
                      {entry.capabilities.length ? (
                        <span className="plugin-caps">
                          {describeCapabilities(entry.capabilities).join(" · ")}
                        </span>
                      ) : null}
                    </div>
                    <div className="plugin-row-actions">
                      <button
                        type="button"
                        className={`pref-switch${entry.enabled ? " is-on" : ""}`}
                        role="switch"
                        aria-checked={entry.enabled}
                        aria-label={`${entry.enabled ? "禁用" : "启用"} ${entry.name}`}
                        onClick={() => toggleInstalled(entry.id, entry.enabled)}
                      >
                        <span className="pref-switch-knob" />
                      </button>
                      <button
                        type="button"
                        className="wg-text-btn is-danger"
                        onClick={() => deleteInstalled(entry.id, entry.name)}
                      >
                        删除
                      </button>
                    </div>
                    {entry.enabled && entry.settings && entry.settings.length > 0 ? (
                      <PluginSettingsForm
                        pluginId={entry.id}
                        fields={entry.settings}
                        description={entry.settingsIntro}
                      />
                    ) : null}
                  </div>
                ))}
              </div>

              <h3 className="plugin-subhead">脚本启动器</h3>
              <p className="settings-lead">
                登记本机 Companion 脚本（独立进程，不注入 Window Hub）。可设置路径、运行环境、关联插件，以及随 Hub
                / 开机启动。
              </p>
              <div className="launcher-form">
                <label className="launcher-field">
                  <span>名称</span>
                  <input
                    type="text"
                    value={launcherDraft.name}
                    placeholder="显示名称"
                    onChange={(e) =>
                      setLauncherDraft((d) => ({ ...d, name: e.target.value }))
                    }
                  />
                </label>
                <label className="launcher-field is-wide">
                  <span>脚本路径</span>
                  <div className="launcher-path-row">
                    <input
                      type="text"
                      value={launcherDraft.scriptPath}
                      placeholder="选择 .py / .js / .ps1 / .exe …"
                      onChange={(e) =>
                        setLauncherDraft((d) => ({ ...d, scriptPath: e.target.value }))
                      }
                    />
                    <button
                      type="button"
                      className="settings-secondary-btn"
                      disabled={launcherBusy}
                      onClick={() => void pickLauncherScript()}
                    >
                      浏览
                    </button>
                  </div>
                </label>
                <label className="launcher-field">
                  <span>运行环境</span>
                  <select
                    value={launcherDraft.environment}
                    onChange={(e) =>
                      setLauncherDraft((d) => ({
                        ...d,
                        environment: e.target.value as ScriptEnv,
                      }))
                    }
                  >
                    {SCRIPT_ENVS.map((env) => (
                      <option key={env.id} value={env.id}>
                        {env.label}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="launcher-field">
                  <span>运行时路径（可选）</span>
                  <input
                    type="text"
                    value={launcherDraft.envPath}
                    placeholder={
                      launcherDraft.environment === "custom"
                        ? "必填：解释器/运行时绝对路径"
                        : "留空则用 PATH 中的 python / node …"
                    }
                    onChange={(e) =>
                      setLauncherDraft((d) => ({ ...d, envPath: e.target.value }))
                    }
                  />
                </label>
                <label className="launcher-field">
                  <span>关联插件</span>
                  <select
                    value={launcherDraft.pluginId}
                    onChange={(e) =>
                      setLauncherDraft((d) => ({ ...d, pluginId: e.target.value }))
                    }
                  >
                    <option value="">不关联</option>
                    {pluginEntries.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                        {p.dev ? " (dev)" : ""}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="launcher-field">
                  <span>额外参数</span>
                  <input
                    type="text"
                    value={launcherDraft.args}
                    placeholder="可选 CLI 参数"
                    onChange={(e) =>
                      setLauncherDraft((d) => ({ ...d, args: e.target.value }))
                    }
                  />
                </label>
                <div className="launcher-checks">
                  <label className="launcher-check">
                    <input
                      type="checkbox"
                      checked={launcherDraft.startWithHub}
                      onChange={(e) =>
                        setLauncherDraft((d) => ({
                          ...d,
                          startWithHub: e.target.checked,
                        }))
                      }
                    />
                    随 Window Hub 启动
                  </label>
                  <label className="launcher-check">
                    <input
                      type="checkbox"
                      checked={launcherDraft.startOnBoot}
                      onChange={(e) =>
                        setLauncherDraft((d) => ({
                          ...d,
                          startOnBoot: e.target.checked,
                        }))
                      }
                    />
                    开机自启（用户 Startup）
                  </label>
                  <label className="launcher-check">
                    <input
                      type="checkbox"
                      checked={launcherDraft.enabled}
                      onChange={(e) =>
                        setLauncherDraft((d) => ({ ...d, enabled: e.target.checked }))
                      }
                    />
                    启用
                  </label>
                </div>
                <div className="plugin-actions">
                  <button
                    type="button"
                    className="settings-primary-btn"
                    disabled={launcherBusy}
                    onClick={() => void saveLauncher()}
                  >
                    {launcherDraft.id ? "保存修改" : "添加启动器"}
                  </button>
                  {launcherDraft.id ? (
                    <button
                      type="button"
                      className="settings-secondary-btn"
                      disabled={launcherBusy}
                      onClick={() => {
                        setLauncherDraft(emptyLauncherDraft());
                        setLauncherMsg("");
                      }}
                    >
                      取消编辑
                    </button>
                  ) : null}
                </div>
              </div>
              {launcherMsg ? <p className="plugin-msg">{launcherMsg}</p> : null}
              <div className="plugin-list">
                {launchers.length === 0 ? (
                  <p className="settings-lead">暂无脚本启动器</p>
                ) : (
                  launchers.map((row) => (
                    <div
                      key={row.id}
                      className={`plugin-row${row.enabled ? "" : " is-disabled"}`}
                    >
                      <div className="plugin-meta">
                        <strong>
                          {row.name}
                          {row.running ? " · 运行中" : ""}
                        </strong>
                        <span>
                          {row.environment}
                          {row.pluginId ? ` · 关联 ${row.pluginId}` : " · 未关联插件"}
                          {row.startWithHub ? " · 随 Hub" : ""}
                          {row.startOnBoot ? " · 开机" : ""}
                        </span>
                        <span className="launcher-path-preview" title={row.scriptPath}>
                          {row.scriptPath}
                        </span>
                      </div>
                      <div className="plugin-row-actions">
                        <button
                          type="button"
                          className="settings-secondary-btn"
                          disabled={launcherBusy}
                          onClick={() => editLauncher(row)}
                        >
                          编辑
                        </button>
                        {row.running ? (
                          <button
                            type="button"
                            className="settings-secondary-btn"
                            disabled={launcherBusy}
                            onClick={() => void runLauncher(row.id, false)}
                          >
                            停止
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="settings-secondary-btn"
                            disabled={launcherBusy || !row.enabled}
                            onClick={() => void runLauncher(row.id, true)}
                          >
                            启动
                          </button>
                        )}
                        <button
                          type="button"
                          className="wg-text-btn is-danger"
                          disabled={launcherBusy}
                          onClick={() => void removeLauncher(row.id, row.name)}
                        >
                          删除
                        </button>
                      </div>
                    </div>
                  ))
                )}
              </div>
            </section>
          )}

          {nav === "developer" && <SqliteDevPanel />}
        </div>
      </main>

      {installPending ? (
        <div
          className="plugin-install-overlay"
          role="dialog"
          aria-modal="true"
          aria-labelledby="plugin-install-title"
        >
          <div className="plugin-install-dialog">
            <h2 id="plugin-install-title">确认安装插件</h2>
            <p className="plugin-install-lead">
              <strong>{installPending.preview.name}</strong>
              <span>
                {" "}
                · {installPending.preview.id} · v{installPending.preview.version}
              </span>
            </p>
            {installPending.preview.description ? (
              <p className="plugin-install-desc">{installPending.preview.description}</p>
            ) : null}

            <h3 className="plugin-install-section">使用的界面</h3>
            <ul className="plugin-install-list">
              {installPending.preview.surfaces.map((s) => (
                <li key={s.id}>
                  <strong>{s.label}</strong>
                  <span>{s.detail}</span>
                </li>
              ))}
            </ul>

            <h3 className="plugin-install-section">声明的能力</h3>
            {installPending.preview.capabilities.length ? (
              <ul className="plugin-install-list">
                {installPending.preview.capabilities.map((cap) => {
                  const c = cap as PluginCapability;
                  const sensitive = isSensitiveCapability(c);
                  return (
                    <li key={cap} className={sensitive ? "is-sensitive" : undefined}>
                      <strong>{describeCapabilities([c])[0] ?? cap}</strong>
                      <span>{sensitive ? "敏感 · 请确认是否信任此插件" : cap}</span>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <p className="plugin-install-empty">未声明 capabilities</p>
            )}

            {installPending.preview.networkHosts.length ? (
              <>
                <h3 className="plugin-install-section">网络白名单</h3>
                <ul className="plugin-install-list">
                  {installPending.preview.networkHosts.map((host) => (
                    <li key={host}>
                      <strong>{host}</strong>
                      <span>hub.fetch 可访问</span>
                    </li>
                  ))}
                </ul>
              </>
            ) : null}

            <div className="plugin-install-actions">
              <button
                type="button"
                className="settings-ghost-btn"
                disabled={pluginBusy}
                onClick={() => setInstallPending(null)}
              >
                取消
              </button>
              <button
                type="button"
                className="settings-primary-btn"
                disabled={pluginBusy}
                onClick={() => void confirmInstallPending()}
              >
                {pluginBusy ? "安装中…" : "确认安装"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
