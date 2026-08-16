import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getIslandPrefs,
  hydrateIslandPrefs,
  setIslandPrefs,
  type IslandPrefs,
} from "./islandPrefs";
import {
  getScenarioGate,
} from "./scenarioGates";
import { listPanelProviders } from "./plugins/panelProviders";
import { listBarResidentProviders, listScenarioProviders } from "./plugins/islandSlots";
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
import PluginsMarketPanel from "./PluginsMarketPanel";
import PrefSelect from "./components/PrefSelect";

type AmbientMode = "edge" | "center";
type DarkPref = "auto" | "dark" | "light";
type NavId = "general" | "theme" | "dock" | "tray" | "plugins" | "developer";

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
  /** Glass strip corner radius in logical px (0 = square). */
  cornerRadiusPx: number;
  /** Overflow-hidden pin ids (restore via dock right-click). */
  hiddenItemIds: string[];
};

type AutostartBackend = "service" | "task" | "none";

type GeneralPrefs = {
  startOnBoot: boolean;
  startOnBootBackend: AutostartBackend;
  /** Optional toast after one-shot UAC for service install/uninstall. */
  notice?: string | null;
};

const AUTOSTART_OPTIONS: { id: AutostartBackend; label: string; desc: string }[] = [
  { id: "none", label: "关闭", desc: "不开机自启；会清除服务与计划任务残留" },
  {
    id: "task",
    label: "计划任务（推荐）",
    desc: "登录时启动，无需管理员；稳定且不影响从资源管理器拖放文件",
  },
  {
    id: "service",
    label: "系统服务",
    desc: "安装/卸载时临时请求管理员（会提示并弹 UAC）；日常以普通权限运行",
  },
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

/** Host-fixed geometry — not exposed in Settings. */
const DOCK_FIXED = {
  activationPosition: "screenBottom" as const,
  activationThicknessPx: 20,
  bottomOffsetPx: 0,
  cornerRadiusPx: 20,
};

function normalizeDockPrefs(dp: Partial<DockPrefs> | null | undefined): DockPrefs {
  return {
    enabled: !!dp?.enabled,
    displayMode: dp?.displayMode || "default",
    hideSystemTaskbar: dp?.hideSystemTaskbar !== false,
    items: dp?.items ?? [],
    hotkey: dp?.hotkey || "Ctrl+Alt+D",
    ...DOCK_FIXED,
    hideLingerMs: Math.min(10000, Math.max(200, Number(dp?.hideLingerMs) || 800)),
    magnification: Math.min(
      2.5,
      Math.max(1, Number.isFinite(Number(dp?.magnification)) ? Number(dp?.magnification) : 1.6),
    ),
    hiddenItemIds: Array.isArray(dp?.hiddenItemIds) ? dp.hiddenItemIds.map(String) : [],
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

type TrayIconInfo = {
  id: string;
  pin_key?: string;
  tooltip: string;
  process: string;
  uid: number;
  hwnd: number;
  callback_msg: number;
  version?: number;
  icon_png_base64: string;
  area: string;
  flashing?: boolean;
  /** IME / input language — forced 常显 */
  resident?: boolean;
};

type TrayPrefs = {
  pinned: string[];
  /** pin_key → 右键菜单高度；未设置则自动 */
  menu_heights?: Record<string, number>;
};

function trayPinKey(icon: TrayIconInfo): string {
  const k = (icon.pin_key || "").trim();
  return k || icon.id;
}

function isTrayPinned(icon: TrayIconInfo, pinned: Set<string>): boolean {
  return pinned.has(trayPinKey(icon)) || pinned.has(icon.id);
}

function isTrayResident(icon: TrayIconInfo): boolean {
  if (icon.resident) return true;
  const tip = (icon.tooltip || "").trim();
  const tipL = tip.toLowerCase();
  const proc = (icon.process || "").trim().toLowerCase();
  const key = (icon.pin_key || icon.id || "").trim().toLowerCase();
  if (
    key === "a59b00b9-f6cd-4fed-a1dc-0f4064a12831" ||
    key === "2c77a81e-41cc-4178-a3a7-5f8a987568e6"
  ) {
    return true;
  }
  if (
    proc === "textinputhost" ||
    proc === "ctfmon" ||
    proc === "tabtip" ||
    proc.includes("sogou") ||
    proc.includes("inputmethod")
  ) {
    return true;
  }
  if (/输入法|语言|ime|language|微软拼音|搜狗|中文/.test(tipL) || tipL.includes("chinese")) {
    return true;
  }
  if (/^[\u4e00-\u9fff]$/.test(tip)) return true;
  return /^(en|eng|chs|cht|jp|jpn|kr|kor|中|英|日|韩)$/i.test(tip);
}

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
  const custom = heights[trayPinKey(icon)] ?? heights[icon.id];
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
  const [ambient, setAmbient] = useState<Ambient>({ r: 32, g: 32, b: 34 });
  const [darkPref, setDarkPref] = useState<DarkPref>("dark");
  const [trays, setTrays] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [menuHeights, setMenuHeights] = useState<Record<string, number>>({});
  /** Drill-down into a tray icon settings card (list → detail). */
  const [trayDetailKey, setTrayDetailKey] = useState<string | null>(null);
  const [menuHeightDraft, setMenuHeightDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [dockPrefs, setDockPrefs] = useState<DockPrefs>(() => normalizeDockPrefs(null));
  const [dockMsg, setDockMsg] = useState("");
  const [dockBusy, setDockBusy] = useState(false);
  const [generalPrefs, setGeneralPrefs] = useState<GeneralPrefs>({
    startOnBoot: false,
    startOnBootBackend: "none",
  });

  const normalizeGeneralPrefs = (gp: Partial<GeneralPrefs> & { startOnBoot?: boolean }): GeneralPrefs => {
    const backend: AutostartBackend =
      gp.startOnBootBackend === "service" || gp.startOnBootBackend === "task" || gp.startOnBootBackend === "none"
        ? gp.startOnBootBackend
        : gp.startOnBoot
          ? "task"
          : "none";
    return {
      startOnBoot: backend !== "none",
      startOnBootBackend: backend,
      notice: typeof gp.notice === "string" ? gp.notice : null,
    };
  };
  const [generalMsg, setGeneralMsg] = useState("");
  const [generalBusy, setGeneralBusy] = useState(false);
  const [islandPrefs, setIslandPrefsState] = useState<IslandPrefs>(() => getIslandPrefs());
  /** scenario pluginId → whether openTrayKey is bound (plugin settings). */
  const [scenarioOpenBound, setScenarioOpenBound] = useState<Record<string, boolean>>({});
  const [shortcutsExclusiveId, setShortcutsExclusiveId] = useState<string>("");
  const [installed, setInstalled] = useState<InstalledPluginDto[]>([]);
  const [pluginMsg, setPluginMsg] = useState("");
  const [pluginBusy, setPluginBusy] = useState(false);
  const [installPending, setInstallPending] = useState<InstallPending | null>(null);
  const [, bumpRegistry] = useState(0);
  const [launchers, setLaunchers] = useState<ScriptLauncherRow[]>([]);
  const [launcherDraft, setLauncherDraft] = useState(emptyLauncherDraft);
  const [launcherMsg, setLauncherMsg] = useState("");
  const [launcherBusy, setLauncherBusy] = useState(false);
  const [focusPluginId, setFocusPluginId] = useState<string | null>(() => {
    const fromWin =
      typeof window !== "undefined" && typeof window.__WH_SETTINGS_FOCUS_PLUGIN__ === "string"
        ? window.__WH_SETTINGS_FOCUS_PLUGIN__.trim()
        : "";
    return fromWin || null;
  });

  useEffect(() => {
    let un: (() => void) | undefined;
    void listen<string>("settings-focus-plugin", (ev) => {
      const id = typeof ev.payload === "string" ? ev.payload.trim() : "";
      if (!id) return;
      setFocusPluginId(id);
      setNav("plugins");
    }).then((fn) => {
      un = fn;
    });
    if (focusPluginId) setNav("plugins");
    return () => un?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- seed nav once from init focus
  }, []);

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

  const scenarioOptions = useMemo(() => {
    return listScenarioProviders();
  }, [installed, bumpRegistry]);

  useEffect(() => {
    let cancelled = false;
    const ids = scenarioOptions.map((o) => o.id);
    void (async () => {
      const next: Record<string, boolean> = {};
      await Promise.all(
        ids.map(async (id) => {
          try {
            const key = await invoke<string | null>("hub_island_get_bound_tray", {
              pluginId: id,
            });
            next[id] = typeof key === "string" && key.trim().length > 0;
          } catch {
            next[id] = false;
          }
        }),
      );
      if (!cancelled) setScenarioOpenBound(next);
    })();
    let un: (() => void) | undefined;
    void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
      "plugin-settings-changed",
      (ev) => {
        const pid = ev.payload?.pluginId;
        if (!pid || !ids.includes(pid)) return;
        const key = String(ev.payload?.settings?.openTrayKey ?? "").trim();
        setScenarioOpenBound((prev) => ({ ...prev, [pid]: Boolean(key) }));
      },
    ).then((fn) => {
      un = fn;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, [scenarioOptions]);

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

  const persistShortcutsExclusive = async (pluginId: string) => {
    setShortcutsExclusiveId(pluginId);
    try {
      const next = await invoke<{ exclusivePluginId?: string | null }>("set_shortcuts_prefs", {
        prefs: { exclusivePluginId: pluginId || null },
      });
      setShortcutsExclusiveId(next.exclusivePluginId ?? "");
    } catch (err) {
      console.error(err);
    }
  };

  const persistDockPrefs = async (patch: Partial<DockPrefs>) => {
    const next: DockPrefs = normalizeDockPrefs({ ...dockPrefs, ...patch, ...DOCK_FIXED });
    setDockPrefs(next);
    setDockBusy(true);
    setDockMsg("");
    try {
      const saved = await invoke<DockPrefs>("set_dock_prefs", { prefs: next });
      setDockPrefs(normalizeDockPrefs(saved));
    } catch (err) {
      console.error(err);
      setDockMsg(String(err));
    } finally {
      setDockBusy(false);
    }
  };

  const persistGeneralPrefs = async (
    patch: Partial<Pick<GeneralPrefs, "startOnBootBackend">>,
  ) => {
    const next = {
      startOnBootBackend: patch.startOnBootBackend ?? generalPrefs.startOnBootBackend,
      startOnBoot: (patch.startOnBootBackend ?? generalPrefs.startOnBootBackend) !== "none",
      runAsAdmin: false,
    };
    setGeneralBusy(true);
    setGeneralMsg("");
    try {
      const saved = normalizeGeneralPrefs(await invoke<GeneralPrefs>("set_general_prefs", { prefs: next }));
      setGeneralPrefs(saved);
      if (saved.notice) {
        setGeneralMsg(saved.notice);
      }
    } catch (err) {
      console.error(err);
      setGeneralMsg(String(err));
    } finally {
      setGeneralBusy(false);
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

  useEffect(() => {
    void syncGlassCss({
      kind: "mica-alt",
      dark: darkPref === "auto" ? null : darkPref === "dark",
    });

    void (async () => {
      await hydrateIslandPrefs().then(setIslandPrefsState);

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
        setMenuHeights(prefs.menu_heights ?? {});
      } catch {
        /* noop */
      }
      try {
        const sp = await invoke<{ exclusivePluginId?: string | null }>("get_shortcuts_prefs");
        setShortcutsExclusiveId(sp.exclusivePluginId ?? "");
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
        const gp = await invoke<GeneralPrefs>("get_general_prefs");
        setGeneralPrefs(normalizeGeneralPrefs(gp));
      } catch {
        /* noop */
      }
    })();

    const unsubs: Array<() => void> = [];
    void listen<Ambient>("ambient-color", (ev) => {
      setAmbient(ev.payload);
    }).then((fn) => unsubs.push(fn));
    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setTrays(ev.payload);
    }).then((fn) => unsubs.push(fn));
    void listen<TrayPrefs>("tray-prefs", (ev) => {
      setPinned(ev.payload.pinned ?? []);
      setMenuHeights(ev.payload.menu_heights ?? {});
    }).then((fn) => unsubs.push(fn));
    void listen<{ exclusivePluginId?: string | null }>("shortcuts-prefs", (ev) => {
      setShortcutsExclusiveId(ev.payload?.exclusivePluginId ?? "");
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

    const poll = window.setInterval(() => {
      void invoke<TrayIconInfo[]>("list_tray_icons")
        .then((list) => setTrays(list))
        .catch(() => undefined);
    }, 800);

    return () => {
      window.clearInterval(poll);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  // Leave tray detail when switching away from the tray tab.
  useEffect(() => {
    if (nav !== "tray") {
      setTrayDetailKey(null);
      setMenuHeightDraft("");
    }
  }, [nav]);

  // If the detailed icon vanished from the live tray list, return to the list.
  useEffect(() => {
    if (!trayDetailKey) return;
    const stillThere = trays.some(
      (t) => trayPinKey(t) === trayDetailKey || t.id === trayDetailKey,
    );
    if (!stillThere) {
      setTrayDetailKey(null);
      setMenuHeightDraft("");
    }
  }, [trays, trayDetailKey]);

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

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
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

  async function persistTrayPrefs(
    nextPinned: string[],
    nextHeights: Record<string, number>,
  ) {
    setSaving(true);
    try {
      const prefs = await invoke<TrayPrefs>("set_tray_prefs", {
        pinned: nextPinned,
        menuHeights: nextHeights,
      });
      setPinned(prefs.pinned ?? nextPinned);
      setMenuHeights(prefs.menu_heights ?? nextHeights);
    } catch {
      /* noop */
    } finally {
      setSaving(false);
    }
  }

  async function togglePinned(icon: TrayIconInfo) {
    // Input language / IME stay resident — cannot unpin.
    if (isTrayResident(icon)) return;
    const key = trayPinKey(icon);
    const next = isTrayPinned(icon, pinnedSet)
      ? pinned.filter((x) => x !== key && x !== icon.id)
      : [...pinned.filter((x) => x !== icon.id), key];
    setPinned(next);
    await persistTrayPrefs(next, menuHeights);
  }

  function openTrayDetail(icon: TrayIconInfo) {
    const key = trayPinKey(icon);
    const cur = menuHeights[key] ?? menuHeights[icon.id];
    setTrayDetailKey(key);
    if (cur != null && cur > 0) {
      setMenuHeightDraft(String(cur));
    } else if (isTencentIm(icon)) {
      setMenuHeightDraft(String(DEFAULT_TENCENT_MENU_HEIGHT));
    } else {
      setMenuHeightDraft("");
    }
  }

  function closeTrayDetail() {
    setTrayDetailKey(null);
    setMenuHeightDraft("");
  }

  async function saveIconMenuHeight(id: string) {
    const raw = menuHeightDraft.trim();
    const parsed = raw === "" ? null : Number(raw);
    const nextHeights = { ...menuHeights };
    // Drop legacy runtime-id entry if present.
    const icon = trays.find((t) => trayPinKey(t) === id || t.id === id);
    if (icon && icon.id !== id) {
      delete nextHeights[icon.id];
    }
    if (parsed == null || !Number.isFinite(parsed) || parsed <= 0) {
      delete nextHeights[id];
    } else {
      nextHeights[id] = Math.round(Math.min(640, Math.max(48, parsed)));
    }
    setMenuHeights(nextHeights);
    await persistTrayPrefs(pinned, nextHeights);
  }

  async function clearIconMenuHeight(id: string) {
    const nextHeights = { ...menuHeights };
    delete nextHeights[id];
    const icon = trays.find((t) => trayPinKey(t) === id || t.id === id);
    if (icon) {
      delete nextHeights[icon.id];
    }
    setMenuHeights(nextHeights);
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
                  推荐「计划任务」：无需管理员、登录即启。系统服务仅在安装/卸载时临时请求管理员（会先提示再弹
                  UAC），日常界面始终普通权限运行。
                </p>
                <div className="pref-row" style={{ marginBottom: 12 }}>
                  <span className="pref-row-text">
                    <span className="pref-row-label">开机自启</span>
                    <span className="pref-row-desc">
                      {AUTOSTART_OPTIONS.find((o) => o.id === generalPrefs.startOnBootBackend)
                        ?.desc ?? ""}
                    </span>
                  </span>
                  <PrefSelect
                    ariaLabel="开机自启"
                    disabled={generalBusy}
                    value={generalPrefs.startOnBootBackend}
                    options={AUTOSTART_OPTIONS.map((o) => ({
                      value: o.id,
                      label: o.label,
                    }))}
                    onChange={(next) =>
                      void persistGeneralPrefs({
                        startOnBootBackend: next as AutostartBackend,
                      })
                    }
                  />
                </div>
                {generalMsg ? <p className="card-desc">{generalMsg}</p> : null}
              </section>
              <section className="settings-card">
                <h2>快捷区</h2>
                <p className="card-desc">
                  状态菜单左侧快捷区可显示多个插件入口，也可独占给某一个插件（例如窗口组固定项占满整条）。
                </p>
                <label className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">快捷区占用</span>
                    <span className="pref-row-desc">选「全部插件」或指定一个 shortcuts 插件</span>
                  </span>
                  <select
                    className="pref-select"
                    value={shortcutsExclusiveId}
                    onChange={(e) => void persistShortcutsExclusive(e.target.value)}
                  >
                    <option value="">全部插件</option>
                    {shortcutsPluginOptions.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                </label>
              </section>
              <section className="settings-card">
                <h2>下拉内容</h2>
                <p className="card-desc">
                  选择点击或下拉展开灵动岛时默认显示的内容。列表来自已启用且声明 island.panel、未设
                  excludeFromPullContent 的插件（如天气、镜子）。中转站等排除项不出现在此，经拖入或岛栏摘要临时打开。
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
                <h2>岛栏常驻</h2>
                <p className="card-desc">
                  折叠态岛栏默认展示哪个插件的摘要。列表来自已启用、声明 island.bar、且非情景临时 /
                  excludeFromBarResident 的插件（如天气）。中转站有条目时仍会临时覆盖；情景插件健康时也会暂代，结束后回到此处选择。
                </p>
                <div className="mode-list">
                  <button
                    type="button"
                    className={`mode-item${islandPrefs.barResident === "" ? " is-selected" : ""}`}
                    onClick={() => updateIslandPrefs({ barResident: "" })}
                  >
                    <span className="mode-label">无</span>
                    <span className="mode-desc">岛栏不常驻任何插件摘要</span>
                  </button>
                  {barResidentOptions.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`mode-item${islandPrefs.barResident === item.id ? " is-selected" : ""}`}
                      onClick={() => updateIslandPrefs({ barResident: item.id })}
                    >
                      <span className="mode-label">{item.label}</span>
                      <span className="mode-desc">{item.description}</span>
                    </button>
                  ))}
                </div>
              </section>
              <section className="settings-card">
                <h2>情景临时</h2>
                <p className="card-desc">
                  启用后，满足条件时自动暂代岛栏摘要与下拉内容（不改动上方常驻/下拉设置）；条件结束（如停播或服务关闭）后自动归还。中转站有暂存条目时仍优先于情景。点进插件可配置存在条件；打开应用绑定在插件设置里。
                </p>
                {scenarioOptions.length === 0 ? (
                  <p className="settings-lead">暂无已启用的情景插件</p>
                ) : (
                  <div className="tray-settings-list">
                    {scenarioOptions.map((item) => {
                      const gate = getScenarioGate(islandPrefs.scenarioGates, item.id);
                      const gateCount = gate.trayKeys.length + gate.windowKeys.length;
                      const openBound = Boolean(scenarioOpenBound[item.id]);
                      return (
                        <button
                          key={item.id}
                          type="button"
                          className="tray-settings-item"
                          onClick={() => {
                            setFocusPluginId(item.id);
                            setNav("plugins");
                          }}
                        >
                          <span className="tray-settings-icon tray-settings-fallback">
                            {item.label.charAt(0)}
                          </span>
                          <span className="tray-settings-meta">
                            <span className="tray-settings-name">{item.label}</span>
                            <span className="tray-settings-sub">
                              {[
                                gateCount > 0
                                  ? `存在条件 ${gateCount} 项`
                                  : "未限制存在条件",
                                openBound ? "已绑定打开托盘" : "未绑定打开托盘",
                              ].join(" · ")}
                            </span>
                          </span>
                          <span className="tray-settings-chevron" aria-hidden>
                            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                              <path
                                d="M6 4l4 4-4 4"
                                stroke="currentColor"
                                strokeWidth="1.5"
                                strokeLinecap="round"
                                strokeLinejoin="round"
                              />
                            </svg>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
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
                  微信等应用托盘图标闪动时，退出沉浸并在岛上落下消息提示（不自动消失）；点击打开应用或左滑均可清掉。展示时岛内描一圈绿色内边框。
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
              </section>
              <section className="settings-card">
                <h2>顶栏采样</h2>
                <p className="card-desc">灵动岛顶栏颜色跟随当前窗口顶部边缘。</p>
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
            </>
          )}

          {nav === "theme" && (
            <section className="settings-card">
              <h2>窗口材质</h2>
              <p className="card-desc">
                设置窗标题栏、左侧与右/下留白共用系统 Mica（随主题深浅）；右侧内容为圆角实色板。托盘 /
                插件弹窗、Dock 仍为磨砂透底。其他 DWMBlurGlass 材质暂未开放。
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
                  Host 自带底栏（非插件）。可导入 MyDockFinder 的 .dockico.ini；图标下方白点表示该应用正在运行。
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
                    <span className="pref-row-label">离开后隐藏延迟</span>
                    <span className="pref-row-desc">
                      鼠标离开 Dock / 激活条后，等待多久再收起（毫秒，默认 800）
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
                  从 MyDockFinder 备份目录选择 `.dockico.ini` 导入。特殊项：开始菜单、回收站、分隔线。
                </p>
                <div className="plugin-actions">
                  <button
                    type="button"
                    className="settings-primary-btn"
                    disabled={dockBusy}
                    onClick={() => void importDockIni()}
                  >
                    {dockBusy ? "处理中…" : "导入 .dockico.ini"}
                  </button>
                </div>
                {dockMsg ? <p className="card-desc">{dockMsg}</p> : null}
              </section>
            </>
          )}

          {nav === "tray" && (() => {
            const detailIcon = trayDetailKey
              ? trays.find(
                  (t) => trayPinKey(t) === trayDetailKey || t.id === trayDetailKey,
                ) ?? null
              : null;

            if (detailIcon) {
              const key = trayPinKey(detailIcon);
              const resident = isTrayResident(detailIcon);
              const on = resident || isTrayPinned(detailIcon, pinnedSet);
              const customH = menuHeights[key] ?? menuHeights[detailIcon.id];
              const tencentDefault = isTencentIm(detailIcon);
              return (
                <section className="settings-card settings-card-grow tray-detail">
                  <button
                    type="button"
                    className="tray-detail-back"
                    onClick={closeTrayDetail}
                  >
                    <svg
                      className="tray-detail-back-icon"
                      width="16"
                      height="16"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.25"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      aria-hidden
                    >
                      <path d="M15 18l-6-6 6-6" />
                    </svg>
                    <span className="tray-detail-back-label">托盘</span>
                  </button>
                  <div className="tray-detail-card">
                    <div className="tray-detail-hero">
                      {detailIcon.icon_png_base64 ? (
                        <img
                          className="tray-detail-hero-icon"
                          src={`data:image/png;base64,${detailIcon.icon_png_base64}`}
                          alt=""
                          draggable={false}
                        />
                      ) : (
                        <span className="tray-detail-hero-icon tray-settings-fallback">
                          {trayLabel(detailIcon).charAt(0).toUpperCase()}
                        </span>
                      )}
                      <div className="tray-detail-hero-meta">
                        <div className="tray-detail-hero-name">{trayLabel(detailIcon)}</div>
                        <div className="tray-detail-hero-sub">
                          {detailIcon.process || detailIcon.id}
                          {resident ? " · 系统常驻" : ""}
                          {detailIcon.area === "overflow" ? " · 溢出区" : ""}
                        </div>
                      </div>
                    </div>

                    <div className="tray-detail-row">
                      <div className="tray-detail-row-text">
                        <div className="tray-detail-row-title">在岛上常显</div>
                        <div className="tray-detail-row-desc">
                          {resident
                            ? "Wi‑Fi / 输入法为系统常驻，不可取消常显"
                            : saving
                              ? "保存中…"
                              : "常显图标会出现在灵动岛托盘区"}
                        </div>
                      </div>
                      <button
                        type="button"
                        className={`pref-switch${on ? " is-on" : ""}${resident ? " is-disabled" : ""}`}
                        role="switch"
                        aria-checked={on}
                        aria-disabled={resident || undefined}
                        disabled={resident}
                        onClick={() => void togglePinned(detailIcon)}
                      >
                        <span className="pref-switch-knob" />
                      </button>
                    </div>

                    <div className="tray-detail-divider" />

                    <div className="tray-detail-block">
                      <div className="tray-detail-row-title">右键菜单高度</div>
                      <div className="tray-detail-row-desc">
                        默认自动测量；可自定义像素高度（48–640）。当前：
                        {menuHeightLabel(detailIcon, menuHeights)}
                      </div>
                      <div className="tray-settings-height tray-detail-height">
                        <input
                          className="pref-input pref-input-sm"
                          type="number"
                          min={48}
                          max={640}
                          placeholder={
                            tencentDefault ? String(DEFAULT_TENCENT_MENU_HEIGHT) : "自动"
                          }
                          value={menuHeightDraft}
                          onChange={(e) => setMenuHeightDraft(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              void saveIconMenuHeight(key);
                            }
                            if (e.key === "Escape") {
                              closeTrayDetail();
                            }
                          }}
                        />
                        <button
                          type="button"
                          className="settings-ghost-btn"
                          onClick={() => void saveIconMenuHeight(key)}
                        >
                          保存
                        </button>
                        <button
                          type="button"
                          className="settings-ghost-btn"
                          disabled={customH == null && !tencentDefault}
                          onClick={() => void clearIconMenuHeight(key)}
                        >
                          {tencentDefault ? "恢复默认" : "自动"}
                        </button>
                      </div>
                    </div>
                  </div>
                </section>
              );
            }

            return (
              <section className="settings-card settings-card-grow">
                {trays.length === 0 ? (
                  <p className="tray-settings-empty">暂未收到托盘图标</p>
                ) : (
                  <div className="tray-settings-list">
                    {trays.map((icon) => {
                      const resident = isTrayResident(icon);
                      return (
                        <button
                          key={icon.id}
                          type="button"
                          className="tray-settings-item"
                          onClick={() => openTrayDetail(icon)}
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
                            {resident ? (
                              <span className="tray-settings-sub">系统常驻</span>
                            ) : null}
                          </span>
                          <span className="tray-settings-chevron" aria-hidden>
                            ›
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </section>
            );
          })()}

          {nav === "plugins" && (
            <PluginsMarketPanel
              pluginEntries={pluginEntries}
              pluginMsg={pluginMsg}
              pluginBusy={pluginBusy}
              focusPluginId={focusPluginId}
              onClearFocusPlugin={() => setFocusPluginId(null)}
              onBeginInstallFromPath={beginInstallFromPath}
              onBeginInstallExample={beginInstallExample}
              onToggleInstalled={toggleInstalled}
              onDeleteInstalled={deleteInstalled}
              launchers={launchers}
              launcherDraft={launcherDraft}
              setLauncherDraft={setLauncherDraft}
              launcherMsg={launcherMsg}
              launcherBusy={launcherBusy}
              onPickLauncherScript={() => void pickLauncherScript()}
              onSaveLauncher={() => void saveLauncher()}
              onEditLauncher={editLauncher}
              onRunLauncher={(id, start) => void runLauncher(id, start)}
              onRemoveLauncher={(id, name) => void removeLauncher(id, name)}
              onResetLauncherDraft={() => {
                setLauncherDraft(emptyLauncherDraft());
                setLauncherMsg("");
              }}
            />
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
