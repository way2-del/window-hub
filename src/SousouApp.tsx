import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./sousou.css";

type Shortcut = {
  id: string;
  name: string;
  path: string;
  kind?: string;
  iconPng?: string | null;
};

type Tab = {
  id: string;
  name: string;
  icon?: string;
  items: Shortcut[];
  /** Bound folder path — panel lists its contents. */
  folderPath?: string;
};

type DirEntry = {
  name: string;
  path: string;
  isDir: boolean;
  iconPng?: string | null;
  size?: number | null;
  modifiedMs: number;
};

type SousouConfig = {
  enabled: boolean;
  everythingExe: string;
  esExe: string;
  doubleCtrlMs: number;
  hotkeyEnabled: boolean;
  windowWidth: number;
  windowHeight: number;
  activeTabId: string;
  tabs: Tab[];
  homeApps: Shortcut[];
  searchFilter?: SearchFilter;
  hoverSwitchTabs?: boolean;
  closeAfterOpen?: boolean;
};

type SearchFilter = {
  enabled: boolean;
  wholeWord: boolean;
  path: string;
  includeSubfolders: boolean;
  extPresets: string[];
  extCustom: string;
  modified: string;
  sizePreset: string;
  sizeMin: string;
  sizeMax: string;
  sizeMinUnit: string;
  sizeMaxUnit: string;
};

function defaultFilter(): SearchFilter {
  return {
    enabled: true,
    wholeWord: false,
    path: "",
    includeSubfolders: true,
    extPresets: [],
    extCustom: "",
    modified: "",
    sizePreset: "",
    sizeMin: "",
    sizeMax: "",
    sizeMinUnit: "MB",
    sizeMaxUnit: "MB",
  };
}

type AppEntry = {
  id: string;
  name: string;
  path: string;
  target: string;
  iconPng?: string | null;
  initials?: string;
};

type RecentEntry = {
  name: string;
  path: string;
  isDir: boolean;
  iconPng?: string | null;
  modifiedMs: number;
};

type FileHit = {
  name: string;
  path: string;
  fullPath: string;
  size?: number | null;
  modified?: string | null;
  isDir: boolean;
  category: string;
};

type CategoryBucket = {
  id: string;
  label: string;
  total: number;
  items: FileHit[];
};

type SearchResponse = {
  query: string;
  apps: AppEntry[];
  files: CategoryBucket[];
  everything: { running: boolean; message: string };
};

const TOOLS: { id: string; name: string; path: string }[] = [
  { id: "calc", name: "计算器", path: "calc.exe" },
  { id: "paint", name: "画图", path: "mspaint.exe" },
  { id: "wordpad", name: "写字板", path: "write.exe" },
  { id: "notepad", name: "记事本", path: "notepad.exe" },
];

const SEARCH_TABS: { id: string; label: string; fileCat?: string }[] = [
  { id: "best", label: "最佳匹配" },
  { id: "apps", label: "应用" },
  { id: "folder", label: "文件夹", fileCat: "folder" },
  { id: "doc", label: "文档", fileCat: "doc" },
  { id: "image", label: "图片", fileCat: "image" },
  { id: "archive", label: "压缩", fileCat: "archive" },
  { id: "media", label: "音视频", fileCat: "media" },
  { id: "other", label: "其他", fileCat: "other" },
];

function iconSrc(png?: string | null) {
  if (!png) return null;
  return png.startsWith("data:") ? png : `data:image/png;base64,${png}`;
}

/** Crop transparent / solid-color margins so logos fill the tile (WeChat IME etc.). */
const trimIconCache = new Map<string, string>();
/** path → PNG base64 from sousou_resolve_icon (avoids IPC skeleton on remount). */
const iconResolveCache = new Map<string, string>();
/** folder path → last listed entries (instant tab switch). */
const dirListCache = new Map<string, DirEntry[]>();
let iconCacheEpoch = 0;
const iconCacheEpochSubs = new Set<() => void>();

function dirCacheKey(path: string) {
  return path.trim().replace(/[/\\]+$/, "").toLowerCase();
}

function rememberIcon(path: string, b64: string | null | undefined) {
  const key = path.trim();
  if (!key || !b64) return;
  iconResolveCache.set(key, b64);
}

function seedIconsFromEntries(entries: { path?: string; iconPng?: string | null }[]) {
  for (const e of entries) {
    if (e.path && e.iconPng) rememberIcon(e.path, e.iconPng);
  }
}

function clearSousouIconFrontendCache() {
  trimIconCache.clear();
  iconResolveCache.clear();
  dirListCache.clear();
  iconCacheEpoch += 1;
  iconCacheEpochSubs.forEach((fn) => fn());
}

function useIconCacheEpoch() {
  const [epoch, setEpoch] = useState(iconCacheEpoch);
  useEffect(() => {
    const fn = () => setEpoch(iconCacheEpoch);
    iconCacheEpochSubs.add(fn);
    return () => {
      iconCacheEpochSubs.delete(fn);
    };
  }, []);
  return epoch;
}

function trimIconDataUrl(src: string): Promise<string> {
  const hit = trimIconCache.get(src);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.decoding = "async";
    img.onload = () => {
      try {
        const w = img.naturalWidth;
        const h = img.naturalHeight;
        if (w < 8 || h < 8) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        const c = document.createElement("canvas");
        c.width = w;
        c.height = h;
        const ctx = c.getContext("2d", { willReadFrequently: true });
        if (!ctx) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        ctx.drawImage(img, 0, 0);
        const { data } = ctx.getImageData(0, 0, w, h);
        const px = (x: number, y: number) => {
          const i = (y * w + x) * 4;
          return [data[i], data[i + 1], data[i + 2], data[i + 3]] as const;
        };
        const dist2 = (
          a: readonly [number, number, number, number],
          b: readonly [number, number, number, number],
        ) => {
          const dr = a[0] - b[0];
          const dg = a[1] - b[1];
          const db = a[2] - b[2];
          return dr * dr + dg * dg + db * db;
        };
        const corners = [px(0, 0), px(w - 1, 0), px(0, h - 1), px(w - 1, h - 1)];
        const opaque = corners.filter((p) => p[3] > 40);
        let bg: readonly [number, number, number, number] | null = null;
        if (opaque.length >= 3) {
          const ref = opaque[0];
          if (opaque.every((p) => dist2(p, ref) <= 48 * 48 * 3)) bg = ref;
        }
        // Ignore soft drop-shadows that inflate the bbox (tiny-glyph bug).
        const CORE_ALPHA = 96;
        const isContent = (x: number, y: number) => {
          const p = px(x, y);
          if (p[3] < CORE_ALPHA) return false;
          if (!bg) return true;
          return dist2(p, bg) > 24 * 24 * 3;
        };
        let minX = w;
        let minY = h;
        let maxX = 0;
        let maxY = 0;
        let any = false;
        // Step 2 for speed on 256px shells; refine edges after.
        const step = w >= 128 ? 2 : 1;
        for (let y = 0; y < h; y += step) {
          for (let x = 0; x < w; x += step) {
            if (isContent(x, y)) {
              any = true;
              if (x < minX) minX = x;
              if (y < minY) minY = y;
              if (x > maxX) maxX = x;
              if (y > maxY) maxY = y;
            }
          }
        }
        if (!any) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        // Expand by step so we don't clip due to sampling.
        minX = Math.max(0, minX - step);
        minY = Math.max(0, minY - step);
        maxX = Math.min(w - 1, maxX + step);
        maxY = Math.min(h - 1, maxY + step);
        const cw = maxX - minX + 1;
        const ch = maxY - minY + 1;
        const fill = Math.max(cw / w, ch / h);
        if (fill >= 0.92) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        const pad = Math.ceil(Math.max(cw, ch) * 0.06);
        const x0 = Math.max(0, minX - pad);
        const y0 = Math.max(0, minY - pad);
        const x1 = Math.min(w, maxX + 1 + pad);
        const y1 = Math.min(h, maxY + 1 + pad);
        const nw = x1 - x0;
        const nh = y1 - y0;
        if (nw <= 0 || nh <= 0 || (nw === w && nh === h)) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        const out = document.createElement("canvas");
        out.width = nw;
        out.height = nh;
        const octx = out.getContext("2d");
        if (!octx) {
          trimIconCache.set(src, src);
          resolve(src);
          return;
        }
        octx.drawImage(c, x0, y0, nw, nh, 0, 0, nw, nh);
        const url = out.toDataURL("image/png");
        if (trimIconCache.size > 200) trimIconCache.clear();
        trimIconCache.set(src, url);
        resolve(url);
      } catch {
        resolve(src);
      }
    };
    img.onerror = () => resolve(src);
    img.src = src;
  });
}

function formatSize(n?: number | null) {
  if (n == null || Number.isNaN(n)) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(2)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(2)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function pickIconB64(pathKey: string, png?: string | null): string | null {
  if (pathKey) {
    const hit = iconResolveCache.get(pathKey);
    if (hit) return hit;
    if (png) {
      rememberIcon(pathKey, png);
      return png;
    }
    return null;
  }
  return png || null;
}

function Icon({
  png,
  name,
  path,
  className,
}: {
  png?: string | null;
  name: string;
  path?: string;
  className?: string;
}) {
  const pathKey = (path || "").trim();
  const cacheEpoch = useIconCacheEpoch();
  const [resolved, setResolved] = useState<string | null>(() => pickIconB64(pathKey, png));
  const [display, setDisplay] = useState<string | null>(() => {
    const raw = iconSrc(pickIconB64(pathKey, png));
    if (!raw) return null;
    // Prefer trimmed; otherwise show raw immediately (no skeleton while canvas trims).
    return trimIconCache.get(raw) ?? raw;
  });

  useEffect(() => {
    const seeded = pickIconB64(pathKey, png);
    if (seeded) {
      setResolved(seeded);
      return;
    }
    if (!pathKey) {
      setResolved(null);
      return;
    }
    setResolved(null);
    let cancelled = false;
    void invoke<string | null>("sousou_resolve_icon", { path: pathKey })
      .then((b64) => {
        if (cancelled || !b64) return;
        rememberIcon(pathKey, b64);
        setResolved(b64);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [pathKey, png, cacheEpoch]);

  useEffect(() => {
    const raw = iconSrc(resolved);
    if (!raw) {
      setDisplay(null);
      return;
    }
    const trimHit = trimIconCache.get(raw);
    if (trimHit) {
      setDisplay(trimHit);
      return;
    }
    setDisplay(raw);
    let cancelled = false;
    void trimIconDataUrl(raw).then((t) => {
      if (!cancelled) setDisplay(t);
    });
    return () => {
      cancelled = true;
    };
  }, [resolved, cacheEpoch]);

  if (display) {
    return (
      <img
        className={`ss-item-icon ${className || ""}`}
        src={display}
        alt=""
        draggable={false}
        decoding="async"
      />
    );
  }
  // Prefer letter tile over shimmer while IPC runs — less "loading" flash.
  return (
    <div className={`ss-item-icon fallback ${className || ""}`}>
      {(name || "?").slice(0, 1)}
    </div>
  );
}

const TAB_ICON_PRESETS: { id: string; label: string }[] = [
  { id: "home", label: "主页" },
  { id: "apps", label: "应用" },
  { id: "code", label: "编程" },
  { id: "work", label: "工作" },
  { id: "notes", label: "笔记" },
  { id: "folder", label: "文件夹" },
  { id: "optimize", label: "优化" },
  { id: "tools", label: "工具" },
  { id: "shop", label: "电商" },
  { id: "community", label: "社区" },
  { id: "game", label: "游戏" },
  { id: "none", label: "无" },
];

const DEFAULT_TAB_ICON: Record<string, string> = {
  home: "home",
  apps: "apps",
  game: "game",
  code: "code",
  work: "work",
  notes: "notes",
  optimize: "optimize",
  tools: "tools",
  shop: "shop",
  xinwu: "community",
};

const TAB_ICON_COLOR: Record<string, string> = {
  home: "#3ecf8e",
  apps: "#4f8cff",
  code: "#8b5cf6",
  work: "#f59e0b",
  notes: "#06b6d4",
  folder: "#f5b83d",
  optimize: "#14b8a6",
  tools: "#64748b",
  shop: "#f43f5e",
  community: "#ec4899",
  game: "#a855f7",
};

function tabIconColor(icon?: string) {
  const id = (icon || "").trim() || "apps";
  return TAB_ICON_COLOR[id] || "#6b7280";
}

function TabGlyph({ icon, colored = true }: { icon?: string; colored?: boolean }) {
  const id = (icon || "").trim() || "apps";
  if (id === "none") return null;
  const color = colored ? tabIconColor(id) : "currentColor";
  const common = {
    width: 15,
    height: 15,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: color,
    strokeWidth: 2.1,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
  switch (id) {
    case "home":
      return (
        <svg {...common}>
          <path d="M3 11.5 12 4l9 7.5" />
          <path d="M6 10.5V20h12v-9.5" />
        </svg>
      );
    case "apps":
      return (
        <svg {...common}>
          <rect x="4" y="4" width="7" height="7" rx="1.5" fill={`${color}22`} />
          <rect x="13" y="4" width="7" height="7" rx="1.5" fill={`${color}22`} />
          <rect x="4" y="13" width="7" height="7" rx="1.5" fill={`${color}22`} />
          <rect x="13" y="13" width="7" height="7" rx="1.5" fill={`${color}22`} />
        </svg>
      );
    case "code":
      return (
        <svg {...common}>
          <path d="M8 7 3 12l5 5" />
          <path d="M16 7l5 5-5 5" />
          <path d="M13 5 11 19" />
        </svg>
      );
    case "work":
      return (
        <svg {...common}>
          <rect x="3" y="8" width="18" height="12" rx="2" fill={`${color}18`} />
          <path d="M8 8V6a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        </svg>
      );
    case "notes":
      return (
        <svg {...common}>
          <path d="M7 3h8l4 4v14H7z" fill={`${color}18`} />
          <path d="M15 3v4h4" />
          <path d="M10 12h6M10 16h6" />
        </svg>
      );
    case "folder":
      return (
        <svg width={15} height={15} viewBox="0 0 24 24" aria-hidden>
          <path
            d="M3.5 7.2c0-1 .8-1.8 1.8-1.8H9l1.6 1.5h7.6c1 0 1.8.8 1.8 1.8V10H3.5V7.2z"
            fill="#e8a317"
          />
          <path
            d="M3.2 9.6h17.6c.9 0 1.6.7 1.6 1.6v7c0 1.1-.9 2-2 2H4.6c-1.1 0-2-.9-2-2v-6.6c0-1.1.9-2 2-2z"
            fill="#ffc94a"
          />
          <path
            d="M4.2 11.2h15.4c.5 0 .9.4.9.9v1.1H3.3v-1.1c0-.5.4-.9.9-.9z"
            fill="#ffe9a0"
            opacity="0.95"
          />
          <path
            d="M3.2 9.6h17.6c.9 0 1.6.7 1.6 1.6v7c0 1.1-.9 2-2 2H4.6c-1.1 0-2-.9-2-2v-6.6c0-1.1.9-2 2-2z"
            fill="none"
            stroke="#d4920f"
            strokeWidth="0.8"
            opacity="0.35"
          />
        </svg>
      );
    case "optimize":
      return (
        <svg {...common}>
          <path d="M12 3v3M12 18v3M3 12h3M18 12h3" />
          <circle cx="12" cy="12" r="4" fill={`${color}22`} />
        </svg>
      );
    case "tools":
      return (
        <svg {...common}>
          <path d="M14.5 5.5 18 9l-7.5 7.5H7v-3.5L14.5 5.5z" fill={`${color}22`} />
          <path d="M5 19l3-1" />
        </svg>
      );
    case "shop":
      return (
        <svg {...common}>
          <path d="M4 8h16l-1.5 11h-13z" fill={`${color}18`} />
          <path d="M8 8V6a4 4 0 0 1 8 0v2" />
        </svg>
      );
    case "community":
      return (
        <svg {...common}>
          <circle cx="9" cy="9" r="3" fill={`${color}22`} />
          <circle cx="16" cy="10" r="2.5" fill={`${color}18`} />
          <path d="M3.5 19c.8-3 2.8-4.5 5.5-4.5s4.7 1.5 5.5 4.5" />
          <path d="M14 14.5c1.8 0 3.4.8 4.2 2.5" />
        </svg>
      );
    case "game":
      return (
        <svg {...common}>
          <path
            d="M6.5 9.5h11c1.8 0 3.2 1.5 3 3.3l-.6 4.2a2.6 2.6 0 0 1-2.6 2.2h-1.4c-.6 0-1.1-.3-1.4-.8l-.7-1.1H9.2l-.7 1.1c-.3.5-.8.8-1.4.8H5.7a2.6 2.6 0 0 1-2.6-2.2l-.6-4.2c-.2-1.8 1.2-3.3 3-3.3z"
            fill={`${color}18`}
          />
          <path d="M8 13.2v3M6.5 14.7h3" />
          <circle cx="15.2" cy="13.4" r="0.9" fill={color} stroke="none" />
          <circle cx="17.4" cy="15.2" r="0.9" fill={color} stroke="none" />
        </svg>
      );
    default:
      return (
        <svg {...common}>
          <circle cx="12" cy="12" r="8" fill={`${color}18`} />
        </svg>
      );
  }
}

function ensureTabIcons(cfg: SousouConfig): SousouConfig {
  return {
    ...cfg,
    tabs: (cfg.tabs || []).map((t) => ({
      ...t,
      icon: t.icon || DEFAULT_TAB_ICON[t.id] || "apps",
    })),
  };
}

export default function SousouApp() {
  const [cfg, setCfg] = useState<SousouConfig | null>(null);
  const [query, setQuery] = useState("");
  const [activeTab, setActiveTab] = useState("home");
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [recent, setRecent] = useState<RecentEntry[]>([]);
  const [search, setSearch] = useState<SearchResponse | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchCat, setSearchCat] = useState("best");
  const [modalOpen, setModalOpen] = useState(false);
  const [filterOpen, setFilterOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [filter, setFilter] = useState<SearchFilter>(() => defaultFilter());
  const [modalTab, setModalTab] = useState<"app" | "url" | "file" | "folder">("app");
  const [picked, setPicked] = useState<Record<string, AppEntry>>({});
  const [urlDraft, setUrlDraft] = useState("");
  const [toast, setToast] = useState("");
  const [evMsg, setEvMsg] = useState("");
  const [iconCacheStats, setIconCacheStats] = useState<{
    entries: number;
    bytes: number;
  } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<number | null>(null);
  const searchGenRef = useRef(0);

  const showToast = (msg: string) => {
    setToast(msg);
    window.setTimeout(() => setToast(""), 2200);
  };

  const refreshIconCacheStats = useCallback(() => {
    void invoke<{ entries: number; bytes: number }>("sousou_icon_cache_stats")
      .then(setIconCacheStats)
      .catch(() => undefined);
  }, []);

  const clearIconCache = useCallback(async () => {
    try {
      const ic = await invoke<{ entries: number; bytes: number }>("sousou_clear_icon_cache");
      clearSousouIconFrontendCache();
      setIconCacheStats(ic);
      showToast("已清除图标缓存");
    } catch (e) {
      showToast(String(e));
    }
  }, []);

  const persist = async (next: SousouConfig) => {
    const saved = await invoke<SousouConfig>("sousou_set_config", { prefs: next });
    setCfg(saved);
    return saved;
  };

  useEffect(() => {
    void (async () => {
      try {
        let c0 = await invoke<SousouConfig>("sousou_get_config");
        // Seed screenshot categories into prefs_sousou when tabs have no items yet.
        let c = c0;
        const emptyTabs = (c.tabs || []).every((t) => !t.items?.length);
        if (emptyTabs) {
          try {
            c = await invoke<SousouConfig>("sousou_seed_tabs");
          } catch {
            /* ignore */
          }
        }
        setCfg(ensureTabIcons(c));
        seedIconsFromEntries(c.homeApps || []);
        for (const t of c.tabs || []) seedIconsFromEntries(t.items || []);
        setFilter({ ...(c.searchFilter || defaultFilter()) });
        setActiveTab(c.activeTabId || "home");
        requestAnimationFrame(() => inputRef.current?.focus());

        void invoke<{ running: boolean; message: string }>("sousou_everything_status")
          .then((st) => {
            if (!st.running) setEvMsg(st.message);
          })
          .catch(() => undefined);

        void invoke<RecentEntry[]>("sousou_list_recent", {
          limit: 18,
          withIcons: false,
        })
          .then((r) => {
            setRecent(
              r.map((e) => ({
                ...e,
                iconPng: e.iconPng ?? iconResolveCache.get(e.path.trim()) ?? null,
              })),
            );
          })
          .then(() =>
            invoke<RecentEntry[]>("sousou_list_recent", {
              limit: 18,
              withIcons: true,
            }).then((r) => {
              seedIconsFromEntries(r);
              setRecent(r);
            }),
          )
          .catch(() => undefined);
      } catch (e) {
        showToast(String(e));
      }
    })();
  }, []);

  // Rust WindowEvent::DragDrop → reliable path ingest (desktop .lnk / Explorer).
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    void listen<{
      ok: boolean;
      added: number;
      message: string;
      prefs?: SousouConfig | null;
    }>("sousou-drop-result", (ev) => {
      if (cancelled) return;
      const p = ev.payload;
      if (p.prefs) {
        setCfg(ensureTabIcons(p.prefs));
        if (p.prefs.activeTabId) setActiveTab(p.prefs.activeTabId);
      } else {
        void invoke<SousouConfig>("sousou_get_config")
          .then((c) => setCfg(ensureTabIcons(c)))
          .catch(() => undefined);
      }
      if (p.message) showToast(p.message);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        un = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  // Settings (or elsewhere) cleared Rust icon cache — drop frontend maps too.
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    void listen("sousou-icon-cache-cleared", () => {
      if (cancelled) return;
      clearSousouIconFrontendCache();
      refreshIconCacheStats();
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        un = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      un?.();
    };
  }, [refreshIconCacheStats]);

  const loadPickerApps = useCallback(async () => {
    if (apps.length > 0) return;
    try {
      const bare = await invoke<AppEntry[]>("sousou_list_apps", {
        withIcons: false,
        limit: 0,
      });
      setApps(
        bare.map((a) => ({
          ...a,
          iconPng:
            a.iconPng ??
            iconResolveCache.get((a.target || a.path).trim()) ??
            null,
        })),
      );
      // Icons in background — don't block picker UI
      void invoke<AppEntry[]>("sousou_list_apps", {
        withIcons: true,
        limit: 80,
      }).then((list) => {
        seedIconsFromEntries(
          list.map((a) => ({ path: a.target || a.path, iconPng: a.iconPng })),
        );
        setApps(list);
      });
    } catch (e) {
      showToast(String(e));
    }
  }, [apps.length]);

  const runSearch = useCallback(async (q: string) => {
    const trimmed = q.trim();
    if (!trimmed) {
      searchGenRef.current += 1;
      setSearch(null);
      setSearchLoading(false);
      return;
    }
    const gen = ++searchGenRef.current;
    setSearchLoading(true);
    try {
      const res = await invoke<SearchResponse>("sousou_search", {
        query: trimmed,
        perCategory: 30,
      });
      if (gen !== searchGenRef.current) return;
      setSearch(res);
      if (!res.everything.running) setEvMsg(res.everything.message);
      else setEvMsg("");
    } catch (e) {
      if (gen !== searchGenRef.current) return;
      showToast(String(e));
      setSearch({
        query: trimmed,
        apps: [],
        files: [],
        everything: { running: false, message: String(e) },
      });
    } finally {
      if (gen === searchGenRef.current) setSearchLoading(false);
    }
  }, []);

  useEffect(() => {
    if (debounceRef.current) window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => {
      void runSearch(query);
    }, 220);
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
  }, [query, runSearch]);

  const openPath = async (path: string) => {
    try {
      await invoke("sousou_open_path", { path });
      if (cfg?.closeAfterOpen !== false) {
        void invoke("sousou_hide").catch(() => undefined);
      }
    } catch (e) {
      showToast(String(e));
    }
  };

  const revealPath = async (path: string) => {
    try {
      await invoke("sousou_reveal_path", { path });
      if (cfg?.closeAfterOpen !== false) {
        void invoke("sousou_hide").catch(() => undefined);
      }
    } catch (e) {
      showToast(String(e));
    }
  };

  const openSystem = async (id: string) => {
    try {
      await invoke("sousou_open_system", { id });
      if (cfg?.closeAfterOpen !== false) {
        void invoke("sousou_hide").catch(() => undefined);
      }
    } catch (e) {
      showToast(String(e));
    }
  };

  const isFilesystemPath = (path: string, kind?: string) => {
    if ((kind || "").toLowerCase() === "url") return false;
    const p = (path || "").trim().toLowerCase();
    if (!p) return false;
    return !(
      p.startsWith("http://") ||
      p.startsWith("https://") ||
      p.startsWith("ms-settings:") ||
      p.startsWith("shell:")
    );
  };

  const ensureEv = async () => {
    try {
      const st = await invoke<{ running: boolean; message: string }>("sousou_ensure_everything");
      setEvMsg(st.running ? "" : st.message);
      showToast(st.running ? "Everything 已启动" : st.message);
      if (query.trim()) void runSearch(query);
    } catch (e) {
      showToast(String(e));
    }
  };

  const currentTab = useMemo(
    () => cfg?.tabs.find((t) => t.id === activeTab) || cfg?.tabs[0],
    [cfg, activeTab],
  );

  const homeApps = useMemo(() => cfg?.homeApps ?? [], [cfg]);

  const saveFilter = async (next: SearchFilter) => {
    setFilter(next);
    if (!cfg) return;
    await persist({ ...cfg, searchFilter: next });
    if (query.trim()) void runSearch(query);
  };

  const filterActive =
    filter.enabled &&
    (!!filter.path.trim() ||
      filter.wholeWord ||
      filter.extPresets.length > 0 ||
      !!filter.extCustom.trim() ||
      !!filter.modified ||
      !!filter.sizePreset ||
      !!filter.sizeMin.trim() ||
      !!filter.sizeMax.trim());

  const toggleExt = (ext: string) => {
    setFilter((f) => {
      const has = f.extPresets.includes(ext);
      return {
        ...f,
        extPresets: has ? f.extPresets.filter((x) => x !== ext) : [...f.extPresets, ext],
      };
    });
  };

  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [tabMenu, setTabMenu] = useState<{
    x: number;
    y: number;
    id: string;
    mode?: "main" | "icons";
  } | null>(null);
  /** Right-click on folder panel content. */
  const [folderPanelMenu, setFolderPanelMenu] = useState<{ x: number; y: number } | null>(null);
  /** Right-click on a pinned shortcut. */
  const [itemMenu, setItemMenu] = useState<{
    x: number;
    y: number;
    id: string;
    scope: "home" | string;
  } | null>(null);
  const [addMenu, setAddMenu] = useState<{ x: number; y: number } | null>(null);
  const [dragTabId, setDragTabId] = useState<string | null>(null);
  const [dragOverTabId, setDragOverTabId] = useState<string | null>(null);
  const [folderDropTabId, setFolderDropTabId] = useState<string | null>(null);
  const [dropBlockedTabId, setDropBlockedTabId] = useState<string | null>(null);
  const [panelDropMode, setPanelDropMode] = useState<"ok" | "blocked" | null>(null);
  const [tabDirEntries, setTabDirEntries] = useState<DirEntry[]>([]);
  const [tabBrowsePath, setTabBrowsePath] = useState<string>("");
  const [tabDirLoading, setTabDirLoading] = useState(false);
  const [dragItemId, setDragItemId] = useState<string | null>(null);
  const [dragOverItemId, setDragOverItemId] = useState<string | null>(null);
  /** Tauri dragDropEnabled breaks HTML5 DnD — pointer reorder only. */
  const dragTabIdRef = useRef<string | null>(null);
  const dragItemIdRef = useRef<string | null>(null);
  const suppressClickRef = useRef(false);
  const reorderLastToRef = useRef<string | null>(null);
  const reorderUnbindRef = useRef<(() => void) | null>(null);
  const folderDropTabIdRef = useRef<string | null>(null);
  const panelDropModeRef = useRef<"ok" | "blocked" | null>(null);
  const cfgRef = useRef(cfg);
  cfgRef.current = cfg;
  const activeTabRef = useRef(activeTab);
  activeTabRef.current = activeTab;
  const tabBrowsePathRef = useRef(tabBrowsePath);
  tabBrowsePathRef.current = tabBrowsePath;

  const arrayMoveById = <T extends { id: string }>(list: T[], fromId: string, toId: string): T[] | null => {
    const from = list.findIndex((x) => x.id === fromId);
    const to = list.findIndex((x) => x.id === toId);
    if (from < 0 || to < 0 || from === to) return null;
    const next = [...list];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    return next;
  };

  const moveTabBefore = (fromId: string, toId: string) => {
    if (fromId === toId) return;
    setCfg((prev) => {
      if (!prev) return prev;
      const tabs = arrayMoveById(prev.tabs, fromId, toId);
      if (!tabs) return prev;
      const next = { ...prev, tabs };
      cfgRef.current = next;
      return next;
    });
  };

  /** Reorder pinned icons: scope `home` → homeApps; otherwise tabId → tab.items */
  const movePinnedBefore = (scope: "home" | string, fromId: string, toId: string) => {
    if (fromId === toId) return;
    setCfg((prev) => {
      if (!prev) return prev;
      if (scope === "home") {
        const homeApps = arrayMoveById(prev.homeApps, fromId, toId);
        if (!homeApps) return prev;
        const next = { ...prev, homeApps };
        cfgRef.current = next;
        return next;
      }
      let changed = false;
      const tabs = prev.tabs.map((t) => {
        if (t.id !== scope) return t;
        const items = arrayMoveById(t.items, fromId, toId);
        if (!items) return t;
        changed = true;
        return { ...t, items };
      });
      if (!changed) return prev;
      const next = { ...prev, tabs };
      cfgRef.current = next;
      return next;
    });
  };

  const stopPointerReorder = (moved: boolean) => {
    reorderUnbindRef.current?.();
    reorderUnbindRef.current = null;
    reorderLastToRef.current = null;
    dragTabIdRef.current = null;
    dragItemIdRef.current = null;
    setDragTabId(null);
    setDragOverTabId(null);
    setDragItemId(null);
    setDragOverItemId(null);
    document.documentElement.classList.remove("ss-reordering");
    if (moved) {
      suppressClickRef.current = true;
      const c = cfgRef.current;
      if (c) void persist(c);
    }
  };

  const beginPointerReorder = (
    e: React.PointerEvent,
    kind: "tab" | "pin",
    id: string,
    scope: string,
  ) => {
    if (e.button !== 0) return;
    // Don't let the browser start native image/text drag.
    e.preventDefault();
    suppressClickRef.current = false;
    reorderUnbindRef.current?.();
    reorderLastToRef.current = null;

    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;
    const pointerId = e.pointerId;
    const dragEl = e.currentTarget as HTMLElement;

    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (!moved && dx * dx + dy * dy < 25) return;
      if (!moved) {
        moved = true;
        document.documentElement.classList.add("ss-reordering");
        if (kind === "tab") {
          dragTabIdRef.current = id;
          setDragTabId(id);
        } else {
          dragItemIdRef.current = id;
          setDragItemId(id);
        }
      }
      ev.preventDefault();

      // Ignore the dragged node so hit-testing sees the target underneath.
      const prevPe = dragEl.style.pointerEvents;
      dragEl.style.pointerEvents = "none";
      const under = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
      dragEl.style.pointerEvents = prevPe;

      if (!under) return;
      if (kind === "tab") {
        const tabEl = under.closest("[data-tab-id]") as HTMLElement | null;
        const toId = tabEl?.getAttribute("data-tab-id");
        if (!toId || toId === id) return;
        if (reorderLastToRef.current === toId) return;
        reorderLastToRef.current = toId;
        moveTabBefore(id, toId);
        setDragOverTabId(toId);
        return;
      }
      const pinEl = under.closest("[data-pin-id]") as HTMLElement | null;
      const toId = pinEl?.getAttribute("data-pin-id");
      if (!toId || toId === id) return;
      if (reorderLastToRef.current === toId) return;
      reorderLastToRef.current = toId;
      movePinnedBefore(scope, id, toId);
      setDragOverItemId(toId);
    };

    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      stopPointerReorder(moved);
    };

    window.addEventListener("pointermove", onMove, { passive: false });
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    reorderUnbindRef.current = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  };

  useEffect(() => {
    return () => {
      reorderUnbindRef.current?.();
    };
  }, []);

  const browseToDir = useCallback((next: string) => {
    const path = next.trim();
    if (!path) return;
    const cached = dirListCache.get(dirCacheKey(path));
    if (cached) {
      setTabDirEntries(cached);
      setTabDirLoading(false);
    } else {
      setTabDirEntries([]);
      setTabDirLoading(true);
    }
    setTabBrowsePath(path);
  }, []);

  const bindFolderToTab = async (tabId: string, folder: string) => {
    const c = cfgRef.current;
    if (!c || tabId === "home") return;
    const path = folder.trim();
    if (!path) return;
    // Folder tag = pure directory view; clear pinned icons.
    const tabs = c.tabs.map((t) =>
      t.id === tabId
        ? {
            ...t,
            folderPath: path,
            items: [],
            icon: t.icon && t.icon !== "none" ? t.icon : "folder",
          }
        : t,
    );
    await persist({ ...c, tabs });
    if (activeTabRef.current === tabId) {
      browseToDir(path);
    }
    showToast("已绑定文件夹");
  };

  const clearFolderOnTab = async (tabId: string) => {
    const c = cfgRef.current;
    if (!c) return;
    const tabs = c.tabs.map((t) =>
      t.id === tabId ? { ...t, folderPath: "" } : t,
    );
    await persist({ ...c, tabs });
    if (activeTabRef.current === tabId) {
      setTabBrowsePath("");
      setTabDirEntries([]);
    }
    showToast("已清除文件夹绑定");
  };

  const addShortcutsToTarget = async (
    target: { type: "home" } | { type: "tab"; tabId: string },
    paths: string[],
  ) => {
    if (!paths.length) return;
    const c = cfgRef.current;
    if (!c) return;
    if (target.type === "tab") {
      const tab = c.tabs.find((t) => t.id === target.tabId);
      if ((tab?.folderPath || "").trim()) {
        showToast("文件夹标签不能添加图标");
        return;
      }
    }
    let items: Shortcut[] = [];
    try {
      items = await invoke<Shortcut[]>("sousou_paths_to_shortcuts", {
        paths,
        withIcons: true,
      });
    } catch (e) {
      showToast(String(e));
      return;
    }
    if (!items.length) {
      showToast("没有可添加的项目");
      return;
    }
    if (target.type === "home") {
      const exist = new Set(c.homeApps.map((i) => i.path.toLowerCase()));
      const merged = [...c.homeApps];
      let n = 0;
      for (const it of items) {
        if (!exist.has(it.path.toLowerCase())) {
          merged.push(it);
          exist.add(it.path.toLowerCase());
          n += 1;
        }
      }
      // Mirror into home tab items if present
      const tabs = c.tabs.map((t) => {
        if (t.id !== "home") return t;
        const ex = new Set(t.items.map((i) => i.path.toLowerCase()));
        const next = [...t.items];
        for (const it of items) {
          if (!ex.has(it.path.toLowerCase())) next.push(it);
        }
        return { ...t, items: next };
      });
      await persist({ ...c, homeApps: merged, tabs });
      showToast(n ? `已添加 ${n} 项` : "已存在，未重复添加");
      return;
    }
    const tabId = target.tabId;
    const tabs = c.tabs.map((t) => {
      if (t.id !== tabId) return t;
      const exist = new Set(t.items.map((i) => i.path.toLowerCase()));
      const next = [...t.items];
      for (const it of items) {
        if (!exist.has(it.path.toLowerCase())) next.push(it);
      }
      return { ...t, items: next };
    });
    await persist({ ...c, tabs });
    const added =
      (tabs.find((t) => t.id === tabId)?.items.length || 0) -
      (c.tabs.find((t) => t.id === tabId)?.items.length || 0);
    showToast(added > 0 ? `已添加 ${added} 项` : "已存在，未重复添加");
  };

  const addShortcutsRef = useRef(addShortcutsToTarget);
  addShortcutsRef.current = addShortcutsToTarget;

  const loadTabDir = useCallback(async (dir: string) => {
    const path = dir.trim();
    if (!path) {
      setTabDirEntries([]);
      setTabDirLoading(false);
      return;
    }
    const key = dirCacheKey(path);
    const cached = dirListCache.get(key);
    if (cached) {
      setTabDirEntries(cached);
      setTabDirLoading(false);
    } else {
      setTabDirLoading(true);
      setTabDirEntries([]);
    }
    const stillHere = () => dirCacheKey(tabBrowsePathRef.current) === key;
    try {
      // Single withIcons pass: Rust disk/mem cache makes this fast; avoids bare→skel→icon flash.
      const full = await invoke<DirEntry[]>("sousou_list_dir", {
        path,
        withIcons: true,
        limit: 200,
      });
      if (!stillHere()) return;
      seedIconsFromEntries(full);
      dirListCache.set(key, full);
      setTabDirEntries(full);
      setTabDirLoading(false);
    } catch (e) {
      if (!stillHere()) return;
      if (!cached) setTabDirEntries([]);
      setTabDirLoading(false);
      showToast(String(e));
    }
  }, []);

  const bindFolderRef = useRef(bindFolderToTab);
  bindFolderRef.current = bindFolderToTab;

  useEffect(() => {
    const bound = (currentTab?.folderPath || "").trim();
    if (activeTab === "home" || !bound) {
      setTabBrowsePath("");
      setTabDirLoading(false);
      // Keep tabDirEntries / dirListCache so returning to a folder tab is instant.
      return;
    }
    const key = dirCacheKey(bound);
    const cached = dirListCache.get(key);
    if (cached) {
      setTabDirEntries(cached);
      setTabDirLoading(false);
    } else if (dirCacheKey(tabBrowsePathRef.current) !== key) {
      // Switching to a different unbound path — avoid showing the previous folder's files.
      setTabDirEntries([]);
      setTabDirLoading(true);
    }
    setTabBrowsePath(bound);
  }, [activeTab, currentTab?.folderPath]);

  useEffect(() => {
    if (!tabBrowsePath.trim()) {
      return;
    }
    void loadTabDir(tabBrowsePath);
  }, [tabBrowsePath, loadTabDir]);

  // Explorer / Desktop / Start Menu → panel (File.path often empty in WebView2).
  // WebviewWindow 内容区拖放会合成到 Window 事件；同时挂 Webview 双通道更稳。
  useEffect(() => {
    let cancelled = false;
    const unFns: Array<() => void> = [];

    type Hit =
      | { kind: "tab"; tabId: string }
      | { kind: "panel" }
      | { kind: "home" }
      | null;

    const activeDropTarget = (): Hit => {
      const aid = activeTabRef.current;
      if (aid === "home") return { kind: "home" };
      return { kind: "panel" };
    };

    const hitFromPoint = async (payload: {
      position?: { x: number; y: number };
    }): Promise<Hit> => {
      const pos = payload.position;
      if (!pos) return activeDropTarget();
      try {
        const factor = await getCurrentWindow().scaleFactor();
        const lx = pos.x / factor;
        const ly = pos.y / factor;
        const el = document.elementFromPoint(lx, ly) as HTMLElement | null;
        if (!el) return activeDropTarget();
        const tabEl = el.closest("[data-tab-id]") as HTMLElement | null;
        if (tabEl) {
          const tabId = tabEl.getAttribute("data-tab-id");
          if (tabId) return { kind: "tab", tabId };
        }
        if (el.closest('[data-ss-drop="home"]')) return { kind: "home" };
        if (el.closest('[data-ss-drop="panel"]')) return { kind: "panel" };
        if (el.closest("[data-ss-drop-zone]")) return activeDropTarget();
        return activeDropTarget();
      } catch {
        return activeDropTarget();
      }
    };

    const applyHover = (hit: Hit) => {
      if (hit?.kind === "tab") {
        folderDropTabIdRef.current = hit.tabId;
        setFolderDropTabId(hit.tabId);
        setDropBlockedTabId(null);
        panelDropModeRef.current = null;
        setPanelDropMode(null);
        return;
      }
      folderDropTabIdRef.current = null;
      setFolderDropTabId(null);
      setDropBlockedTabId(null);
      panelDropModeRef.current = "ok";
      setPanelDropMode("ok");
    };

    const clearHover = () => {
      folderDropTabIdRef.current = null;
      setFolderDropTabId(null);
      setDropBlockedTabId(null);
      panelDropModeRef.current = null;
      setPanelDropMode(null);
    };

    const importIntoFolder = async (dest: string, paths: string[]) => {
      const n = await invoke<number>("sousou_import_into_folder", { dest, paths });
      const destKey = dest.replace(/[/\\]+$/, "").toLowerCase();
      const browseKey = tabBrowsePathRef.current.replace(/[/\\]+$/, "").toLowerCase();
      const boundKey = (
        cfgRef.current?.tabs.find((t) => t.id === activeTabRef.current)?.folderPath || ""
      )
        .replace(/[/\\]+$/, "")
        .toLowerCase();
      if (browseKey === destKey || boundKey === destKey) {
        dirListCache.delete(destKey);
        await loadTabDir(tabBrowsePathRef.current || dest);
      }
      showToast(n > 0 ? `已放入文件夹 ${n} 项` : "没有可放入的项目");
    };

    const ingestPaths = async (paths: string[], hit: Hit) => {
      if (!paths.length) {
        showToast("未读到文件路径（请从资源管理器 / 桌面拖入 .lnk 或 .exe）");
        return;
      }
      const target = hit || activeDropTarget();
      if (!target) return;

      if (target.kind === "tab") {
        const c = cfgRef.current;
        const tab = c?.tabs.find((t) => t.id === target.tabId);
        if (target.tabId === "home") {
          await addShortcutsRef.current({ type: "home" }, paths);
          setActiveTab("home");
          return;
        }
        const bound = (tab?.folderPath || "").trim();
        if (bound) {
          try {
            await importIntoFolder(bound, paths);
          } catch (e) {
            showToast(String(e));
          }
          setActiveTab(target.tabId);
          return;
        }
        const onlyFolder =
          paths.length === 1
            ? await invoke<string | null>("sousou_first_folder", { paths })
            : null;
        const emptyPins = !(tab?.items?.length);
        if (onlyFolder && emptyPins) {
          await bindFolderRef.current(target.tabId, onlyFolder);
          setActiveTab(target.tabId);
          return;
        }
        await addShortcutsRef.current({ type: "tab", tabId: target.tabId }, paths);
        setActiveTab(target.tabId);
        const cur = cfgRef.current;
        if (cur && cur.activeTabId !== target.tabId) {
          void persist({ ...cur, activeTabId: target.tabId });
        }
        return;
      }

      if (target.kind === "home") {
        await addShortcutsRef.current({ type: "home" }, paths);
        return;
      }

      const aid = activeTabRef.current;
      if (aid === "home") {
        await addShortcutsRef.current({ type: "home" }, paths);
        return;
      }
      const tab = cfgRef.current?.tabs.find((t) => t.id === aid);
      const bound = (tab?.folderPath || "").trim();
      if (bound) {
        const dest = (tabBrowsePathRef.current || bound).trim();
        try {
          await importIntoFolder(dest, paths);
        } catch (e) {
          showToast(String(e));
        }
        return;
      }
      await addShortcutsRef.current({ type: "tab", tabId: aid }, paths);
    };

    let dropBusy = false;
    const onDragDrop = (ev: {
      payload: { type: string; paths?: string[]; position?: { x: number; y: number } };
    }) => {
      const p = ev.payload;
      if (p.type === "enter" || p.type === "over") {
        void hitFromPoint(p).then((hit) => {
          if (!cancelled) applyHover(hit);
        });
        return;
      }
      if (p.type === "leave") {
        clearHover();
        return;
      }
      if (p.type !== "drop") return;
      if (dropBusy) return;
      dropBusy = true;
      const tabIdHint = folderDropTabIdRef.current;
      clearHover();
      void (async () => {
        try {
          const paths = ("paths" in p ? p.paths : null) ?? [];
          const hit =
            (await hitFromPoint(p)) ||
            (tabIdHint ? ({ kind: "tab", tabId: tabIdHint } as const) : null) ||
            activeDropTarget();
          await ingestPaths(paths, hit);
        } finally {
          dropBusy = false;
        }
      })();
    };

    const bind = (label: string, promise: Promise<() => void>) => {
      void promise
        .then((fn) => {
          if (cancelled) {
            fn();
            return;
          }
          unFns.push(fn);
        })
        .catch((err) => {
          console.error(`[sousou] onDragDropEvent (${label}) unavailable`, err);
        });
    };

    // WindowContent 拖放事件走 Window；Webview 再挂一份兜底
    bind("window", getCurrentWindow().onDragDropEvent(onDragDrop));
    bind("webview", getCurrentWebview().onDragDropEvent(onDragDrop));

    // HTML5 兜底：不 preventDefault 时系统会显示禁止光标
    const allowHtml5 = (e: DragEvent) => {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
      if (!panelDropModeRef.current) {
        panelDropModeRef.current = "ok";
        setPanelDropMode("ok");
      }
    };
    const onHtml5Leave = (e: DragEvent) => {
      if (e.relatedTarget) return;
      clearHover();
    };
    const onHtml5Drop = (e: DragEvent) => {
      e.preventDefault();
      clearHover();
      const files = e.dataTransfer?.files;
      if (!files?.length) return;
      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        const f = files[i] as File & { path?: string };
        if (f.path) paths.push(f.path);
      }
      if (!paths.length) return;
      void ingestPaths(paths, activeDropTarget());
    };
    document.addEventListener("dragenter", allowHtml5);
    document.addEventListener("dragover", allowHtml5);
    document.addEventListener("dragleave", onHtml5Leave);
    document.addEventListener("drop", onHtml5Drop);

    return () => {
      cancelled = true;
      for (const fn of unFns) fn();
      document.removeEventListener("dragenter", allowHtml5);
      document.removeEventListener("dragover", allowHtml5);
      document.removeEventListener("dragleave", onHtml5Leave);
      document.removeEventListener("drop", onHtml5Drop);
      clearHover();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!tabMenu && !addMenu && !folderPanelMenu && !itemMenu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setTabMenu(null);
        setAddMenu(null);
        setFolderPanelMenu(null);
        setItemMenu(null);
      }
    };
    const onDown = () => {
      setTabMenu(null);
      setAddMenu(null);
      setFolderPanelMenu(null);
      setItemMenu(null);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [tabMenu, addMenu, folderPanelMenu, itemMenu]);

  const removePinned = async (scope: "home" | string, id: string) => {
    const c = cfgRef.current;
    if (!c) return;
    if (scope === "home") {
      const homeApps = c.homeApps.filter((i) => i.id !== id);
      const tabs = c.tabs.map((t) =>
        t.id === "home" ? { ...t, items: t.items.filter((i) => i.id !== id) } : t,
      );
      await persist({ ...c, homeApps, tabs });
    } else {
      const tabs = c.tabs.map((t) =>
        t.id === scope ? { ...t, items: t.items.filter((i) => i.id !== id) } : t,
      );
      await persist({ ...c, tabs });
    }
    setItemMenu(null);
    showToast("已移除");
  };

  const addTab = async () => {
    if (!cfg) return;
    const id = `tab-${Date.now()}`;
    const name = `标签${cfg.tabs.length + 1}`;
    const next = {
      ...cfg,
      tabs: [...cfg.tabs, { id, name, icon: "apps", items: [], folderPath: "" }],
      activeTabId: id,
    };
    await persist(next);
    setActiveTab(id);
    setAddMenu(null);
  };

  const addFolderTab = async () => {
    setAddMenu(null);
    const p = await invoke<string | null>("sousou_pick_folder").catch(() => null);
    if (!p || !cfgRef.current) return;
    const c = cfgRef.current;
    const name = p.split(/[/\\]/).filter(Boolean).pop() || "文件夹";
    const id = `tab-${Date.now()}`;
    const next = {
      ...c,
      tabs: [...c.tabs, { id, name, icon: "folder", items: [], folderPath: p }],
      activeTabId: id,
    };
    await persist(next);
    setActiveTab(id);
    showToast(`已添加「${name}」`);
  };

  const deleteTab = async (tabId: string) => {
    if (!cfg) return;
    if (tabId === "home") {
      showToast("主页标签不可删除");
      setTabMenu(null);
      return;
    }
    if (cfg.tabs.length <= 1) {
      showToast("至少保留一个标签");
      setTabMenu(null);
      return;
    }
    const tabs = cfg.tabs.filter((t) => t.id !== tabId);
    const activeTabId =
      activeTab === tabId ? tabs[0]?.id || "home" : cfg.activeTabId;
    await persist({ ...cfg, tabs, activeTabId });
    if (activeTab === tabId) setActiveTab(activeTabId);
    setTabMenu(null);
    showToast("已删除标签");
  };

  const commitRename = async () => {
    if (!cfg || !renamingId) return;
    const name = renameDraft.trim();
    if (!name) {
      setRenamingId(null);
      return;
    }
    const tabs = cfg.tabs.map((t) => (t.id === renamingId ? { ...t, name } : t));
    await persist({ ...cfg, tabs });
    setRenamingId(null);
  };

  const commitPicked = async () => {
    if (!cfg || !currentTab) return;
    const items: Shortcut[] = Object.values(picked).map((a) => ({
      id: a.id,
      name: a.name,
      path: a.target || a.path,
      kind: "app",
      iconPng: a.iconPng,
    }));
    if (modalTab === "url" && urlDraft.trim()) {
      const u = urlDraft.trim();
      items.push({
        id: `url-${Date.now()}`,
        name: u.replace(/^https?:\/\//, "").slice(0, 24),
        path: u.startsWith("http") ? u : `https://${u}`,
        kind: "url",
      });
    }
    if (!items.length) {
      setModalOpen(false);
      return;
    }
    const tabs = cfg.tabs.map((t) => {
      if (t.id !== currentTab.id) return t;
      const exist = new Set(t.items.map((i) => i.path.toLowerCase()));
      const merged = [...t.items];
      for (const it of items) {
        if (!exist.has(it.path.toLowerCase())) merged.push(it);
      }
      return { ...t, items: merged };
    });
    let homeAppsNext = cfg.homeApps;
    if (currentTab.id === "home" || activeTab === "home") {
      const exist = new Set(homeAppsNext.map((i) => i.path.toLowerCase()));
      homeAppsNext = [...homeAppsNext];
      for (const it of items) {
        if (!exist.has(it.path.toLowerCase())) homeAppsNext.push(it);
      }
    }
    await persist({ ...cfg, tabs, homeApps: homeAppsNext });
    setPicked({});
    setUrlDraft("");
    setModalOpen(false);
    showToast("已添加到当前标签");
  };

  const searching = query.trim().length > 0;
  const resultsFresh = !!search && search.query === query.trim();
  const searchBusy = searching && (searchLoading || !resultsFresh);

  const fileBucket = (id: string) => search?.files.find((f) => f.id === id);

  const renderBest = () => {
    if (!search) return null;
    const hasApps = search.apps.length > 0;
    const hasFiles = (["folder", "doc", "image", "archive", "media", "all"] as const).some(
      (cid) => (fileBucket(cid)?.items?.length ?? 0) > 0,
    );
    if (!hasApps && !hasFiles) {
      const pathFilter = (filter?.enabled && filter.path?.trim()) || "";
      return (
        <div className="ss-empty">
          {search.everything?.running === false
            ? search.everything.message || "Everything 未运行"
            : pathFilter
              ? `无匹配结果（当前筛选限定路径：${pathFilter}，可点漏斗清除）`
              : "无匹配结果"}
        </div>
      );
    }
    return (
      <>
        {hasApps && (
          <>
            <div className="ss-section-label">应用</div>
            <div className="ss-grid">
              {search.apps.slice(0, 12).map((a) => (
                <button
                  key={a.id}
                  type="button"
                  className="ss-item"
                  onClick={() => void openPath(a.target || a.path)}
                >
                  <Icon png={a.iconPng} name={a.name} path={a.target || a.path} />
                  <span className="ss-item-name">{a.name}</span>
                </button>
              ))}
            </div>
          </>
        )}
        {(["folder", "doc", "image", "archive", "media", "all"] as const).map((cid) => {
          const b = fileBucket(cid === "all" ? "all" : cid);
          if (!b?.items?.length) return null;
          return (
            <div key={cid}>
              <div className="ss-section-label">{b.label}</div>
              {cid === "folder" ? (
                <div className="ss-grid recent">
                  {b.items.slice(0, 12).map((f) => (
                    <button
                      key={f.fullPath}
                      type="button"
                      className="ss-item ss-recent-row"
                      onClick={() => void openPath(f.fullPath)}
                    >
                      <Icon name={f.name} path={f.fullPath} />
                      <div className="ss-recent-meta">
                        <div className="name">{f.name}</div>
                        <div className="path">{f.path}</div>
                      </div>
                    </button>
                  ))}
                </div>
              ) : (
                <div className="ss-file-list">
                  {b.items.slice(0, 16).map((f) => (
                    <button
                      key={f.fullPath}
                      type="button"
                      className="ss-file-row"
                      onClick={() => void openPath(f.fullPath)}
                    >
                      <Icon name={f.name} path={f.fullPath} />
                      <span>{f.name}</span>
                      <span className="muted">{f.fullPath}</span>
                      <span className="muted">{formatSize(f.size)}</span>
                      <span className="muted">{f.modified || "—"}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </>
    );
  };

  const renderSearchCat = () => {
    if (!search) return <div className="ss-empty">输入关键词开始搜索</div>;
    if (searchCat === "best") return renderBest();
    if (searchCat === "apps") {
      if (!search.apps.length) return <div className="ss-empty">无匹配应用</div>;
      return (
        <div className="ss-grid">
          {search.apps.map((a) => (
            <button
              key={a.id}
              type="button"
              className="ss-item"
              onClick={() => void openPath(a.target || a.path)}
            >
              <Icon png={a.iconPng} name={a.name} path={a.target || a.path} />
              <span className="ss-item-name">{a.name}</span>
            </button>
          ))}
        </div>
      );
    }
    const meta = SEARCH_TABS.find((t) => t.id === searchCat);
    const b = fileBucket(meta?.fileCat || searchCat);
    if (!b?.items?.length) return <div className="ss-empty">无匹配结果</div>;
    if (searchCat === "folder") {
      return (
        <div className="ss-grid recent">
          {b.items.map((f) => (
            <button
              key={f.fullPath}
              type="button"
              className="ss-item ss-recent-row"
              onClick={() => void openPath(f.fullPath)}
            >
              <Icon name={f.name} path={f.fullPath} />
              <div className="ss-recent-meta">
                <div className="name">{f.name}</div>
                <div className="path">{f.path}</div>
              </div>
            </button>
          ))}
        </div>
      );
    }
    return (
      <div className="ss-file-list">
        {b.items.map((f) => (
          <button
            key={f.fullPath}
            type="button"
            className="ss-file-row"
            onClick={() => void openPath(f.fullPath)}
          >
            <Icon name={f.name} path={f.fullPath} />
            <span>{f.name}</span>
            <span className="muted">{f.fullPath}</span>
            <span className="muted">{formatSize(f.size)}</span>
            <span className="muted">{f.modified || "—"}</span>
          </button>
        ))}
      </div>
    );
  };

  const switchTab = (id: string) => {
    setActiveTab(id);
    setTabMenu(null);
    if (cfg && cfg.activeTabId !== id) void persist({ ...cfg, activeTabId: id });
  };

  const patchPrefs = async (patch: Partial<SousouConfig>) => {
    if (!cfg) return;
    await persist({ ...cfg, ...patch });
  };

  const catCount = (id: string) => {
    if (!search) return "";
    if (id === "apps") return search.apps.length ? ` ${search.apps.length}` : "";
    if (id === "best") return "";
    const b = fileBucket(SEARCH_TABS.find((t) => t.id === id)?.fileCat || id);
    return b && b.total > 0 ? ` ${b.total}` : "";
  };

  const renderPinnedItems = (items: Shortcut[], scope: "home" | string) =>
    items.map((it) => (
      <button
        key={it.id}
        type="button"
        data-pin-id={it.id}
        className={[
          "ss-item",
          "ss-pin",
          dragItemId === it.id ? "is-dragging" : "",
          dragOverItemId === it.id && dragItemId !== it.id ? "is-drag-over" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        onClick={() => {
          if (suppressClickRef.current) {
            suppressClickRef.current = false;
            return;
          }
          void openPath(it.path);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setTabMenu(null);
          setAddMenu(null);
          setFolderPanelMenu(null);
          setItemMenu({ x: e.clientX, y: e.clientY, id: it.id, scope });
        }}
        onPointerDown={(e) => beginPointerReorder(e, "pin", it.id, scope)}
      >
        <Icon png={it.iconPng} name={it.name} path={it.path} />
        <span className="ss-item-name">{it.name}</span>
      </button>
    ));

  return (
    <div className="ss-root">
      <div className="ss-chrome">
      <header className="ss-header">
        <div className={`ss-search${query.trim() ? " has-query" : ""}${filterActive ? " has-filter" : ""}${searchBusy ? " is-busy" : ""}`}>
          <span className={`ss-search-ico${searchBusy ? " is-busy" : ""}`} aria-hidden>
            {searchBusy ? (
              <i className="ss-spinner" />
            ) : (
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2">
                <circle cx="11" cy="11" r="7" />
                <path d="M20 20l-3.5-3.5" />
              </svg>
            )}
          </span>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={"输入 jsq 可以找到「计算器」"}
            spellCheck={false}
            aria-busy={searchBusy}
          />
          {!!query.trim() && (
            <button
              type="button"
              className="ss-search-clear"
              title="清除"
              aria-label="清除搜索"
              onClick={() => {
                setQuery("");
                inputRef.current?.focus();
              }}
            >
              ×
            </button>
          )}
          <div className="ss-search-divider" aria-hidden />
          <button
            type="button"
            className={`ss-search-filter${filterActive ? " is-active" : ""}`}
            title="搜索结果筛选"
            aria-label="筛选"
            onClick={() => setFilterOpen(true)}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M4 5h16l-6 7v5l-4 2v-7L4 5z" />
            </svg>
            <span>筛选</span>
            {filterActive && <i className="ss-search-filter-dot" />}
          </button>
        </div>
        <button
          type="button"
          className="ss-chrome-btn ss-settings-btn"
          title="搜搜设置"
          aria-label="设置"
          onClick={() => {
            setSettingsOpen(true);
            refreshIconCacheStats();
          }}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9c.2.6.8 1 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
          </svg>
        </button>
      </header>

      {!searching && (
        <nav
          className="ss-tabs"
          onClick={() => {
            setTabMenu(null);
            setAddMenu(null);
          }}
        >
          {(cfg?.tabs || []).map((t) =>
            renamingId === t.id ? (
              <input
                key={t.id}
                autoFocus
                value={renameDraft}
                onChange={(e) => setRenameDraft(e.target.value)}
                onBlur={() => void commitRename()}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void commitRename();
                  if (e.key === "Escape") setRenamingId(null);
                }}
                className="ss-tab-rename"
              />
            ) : (
              <button
                key={t.id}
                type="button"
                data-tab-id={t.id}
                title={t.folderPath ? `文件夹: ${t.folderPath}` : undefined}
                className={[
                  "ss-tab",
                  activeTab === t.id ? "active" : "",
                  dragTabId === t.id ? "is-dragging" : "",
                  dragOverTabId === t.id && dragTabId !== t.id ? "is-drag-over" : "",
                  folderDropTabId === t.id && t.id !== "home" ? "is-folder-drop" : "",
                  dropBlockedTabId === t.id ? "is-drop-blocked" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                style={
                  {
                    ["--ss-tab-accent" as string]: tabIconColor(
                      t.icon || DEFAULT_TAB_ICON[t.id],
                    ),
                  } as CSSProperties
                }
                onClick={() => {
                  if (suppressClickRef.current) {
                    suppressClickRef.current = false;
                    return;
                  }
                  switchTab(t.id);
                }}
                onMouseEnter={() => {
                  if (dragTabIdRef.current || reorderUnbindRef.current) return;
                  if (cfg?.hoverSwitchTabs === false) return;
                  if (activeTab === t.id) return;
                  switchTab(t.id);
                }}
                onDoubleClick={() => {
                  setRenamingId(t.id);
                  setRenameDraft(t.name);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setTabMenu({ x: e.clientX, y: e.clientY, id: t.id });
                }}
                onPointerDown={(e) => beginPointerReorder(e, "tab", t.id, "")}
              >
                {t.icon !== "none" && (
                  <span
                    className="ss-tab-glyph"
                    style={{ color: tabIconColor(t.icon || DEFAULT_TAB_ICON[t.id]) }}
                  >
                    <TabGlyph icon={t.icon || DEFAULT_TAB_ICON[t.id]} />
                  </span>
                )}
                {t.name}
              </button>
            ),
          )}
          <div className="ss-tabs-spacer" />
          <button
            type="button"
            className={`ss-icon-btn ss-add-tab-btn${addMenu ? " is-active" : ""}`}
            title="添加标签"
            aria-label="添加标签"
            onClick={(e) => {
              e.stopPropagation();
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              setTabMenu(null);
              setAddMenu((m) =>
                m ? null : { x: Math.max(8, r.right - 228), y: r.bottom + 6 },
              );
            }}
          >
            +
          </button>
        </nav>
      )}
      </div>

      {addMenu && (
        <div
          className="ss-ctx-menu ss-add-menu"
          style={{ left: addMenu.x, top: addMenu.y }}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          <button type="button" className="ss-add-menu-item" onClick={() => void addTab()}>
            <span className="ss-add-menu-ico ss-add-menu-ico-tag" aria-hidden>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
                <path
                  d="M3.5 12.2 11.3 4.4A2 2 0 0 1 12.7 4h5.8A2 2 0 0 1 20.5 6v5.8a2 2 0 0 1-.6 1.4L12 21.1a1.2 1.2 0 0 1-1.7 0L3.5 14a1.2 1.2 0 0 1 0-1.8z"
                  fill="#3ecf8e"
                  opacity="0.22"
                />
                <path
                  d="M3.5 12.2 11.3 4.4A2 2 0 0 1 12.7 4h5.8A2 2 0 0 1 20.5 6v5.8a2 2 0 0 1-.6 1.4L12 21.1a1.2 1.2 0 0 1-1.7 0L3.5 14a1.2 1.2 0 0 1 0-1.8z"
                  stroke="#2db87a"
                  strokeWidth="1.6"
                  strokeLinejoin="round"
                />
                <circle cx="16.2" cy="7.8" r="1.35" fill="#2db87a" />
              </svg>
            </span>
            <span className="ss-add-menu-text">
              <strong>新建标签</strong>
              <small>空白标签，可拖入应用图标</small>
            </span>
          </button>
          <button type="button" className="ss-add-menu-item" onClick={() => void addFolderTab()}>
            <span className="ss-add-menu-ico ss-add-menu-ico-folder" aria-hidden>
              <svg width="20" height="20" viewBox="0 0 24 24">
                <path
                  d="M3.2 7c0-.9.7-1.6 1.6-1.6H9l1.5 1.4h8.2c.9 0 1.6.7 1.6 1.6V10H3.2V7z"
                  fill="#f0a11a"
                />
                <path
                  d="M3 9.8h18c.9 0 1.6.7 1.6 1.6v7c0 1-.8 1.8-1.8 1.8H4.2c-1 0-1.8-.8-1.8-1.8v-6.8c0-1 .8-1.8 1.8-1.8z"
                  fill="#ffcc3d"
                />
                <path d="M3.4 11.4h17.2v1.6H3.4z" fill="#ffe7a0" />
                <path
                  d="M3 9.8h18c.9 0 1.6.7 1.6 1.6v7c0 1-.8 1.8-1.8 1.8H4.2c-1 0-1.8-.8-1.8-1.8v-6.8c0-1 .8-1.8 1.8-1.8z"
                  fill="none"
                  stroke="#d4890c"
                  strokeWidth="0.7"
                  opacity="0.4"
                />
              </svg>
            </span>
            <span className="ss-add-menu-text">
              <strong>添加文件夹</strong>
              <small>选择文件夹，直接浏览内容</small>
            </span>
          </button>
        </div>
      )}

      {tabMenu && (
        <div
          className="ss-ctx-menu"
          style={{ left: tabMenu.x, top: tabMenu.y }}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          {(tabMenu.mode || "main") === "main" ? (
            <>
              <button
                type="button"
                onClick={() => {
                  const t = cfg?.tabs.find((x) => x.id === tabMenu.id);
                  if (t) {
                    setRenamingId(t.id);
                    setRenameDraft(t.name);
                  }
                  setTabMenu(null);
                }}
              >
                重命名
              </button>
              <button
                type="button"
                onClick={() => setTabMenu({ ...tabMenu, mode: "icons" })}
              >
                更换图标
              </button>
              {tabMenu.id !== "home" && (
                <button
                  type="button"
                  onClick={() => {
                    setTabMenu(null);
                    void invoke<string | null>("sousou_pick_folder").then((p) => {
                      if (p) void bindFolderToTab(tabMenu.id, p);
                    });
                  }}
                >
                  {cfg?.tabs.find((t) => t.id === tabMenu.id)?.folderPath
                    ? "更换文件夹"
                    : "绑定文件夹"}
                </button>
              )}
              {!!cfg?.tabs.find((t) => t.id === tabMenu.id)?.folderPath && (
                <button
                  type="button"
                  onClick={() => {
                    const p = cfg?.tabs.find((t) => t.id === tabMenu.id)?.folderPath || "";
                    setTabMenu(null);
                    if (p) void openPath(p);
                  }}
                >
                  打开原文件夹
                </button>
              )}
              {!!cfg?.tabs.find((t) => t.id === tabMenu.id)?.folderPath && (
                <button
                  type="button"
                  onClick={() => {
                    setTabMenu(null);
                    void clearFolderOnTab(tabMenu.id);
                  }}
                >
                  清除文件夹
                </button>
              )}
              <button
                type="button"
                className="danger"
                disabled={tabMenu.id === "home"}
                onClick={() => void deleteTab(tabMenu.id)}
              >
                删除
              </button>
            </>
          ) : (
            <div className="ss-icon-picker">
              <div className="ss-icon-picker-title">选择图标</div>
              <div className="ss-icon-picker-grid">
                {TAB_ICON_PRESETS.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    title={p.label}
                    className={
                      (cfg?.tabs.find((t) => t.id === tabMenu.id)?.icon || "") === p.id
                        ? "active"
                        : ""
                    }
                    onClick={() => {
                      if (!cfg) return;
                      const tabs = cfg.tabs.map((t) =>
                        t.id === tabMenu.id ? { ...t, icon: p.id } : t,
                      );
                      void persist({ ...cfg, tabs });
                      setTabMenu(null);
                    }}
                  >
                    {p.id === "none" ? (
                      <span className="ss-icon-none">无</span>
                    ) : (
                      <TabGlyph icon={p.id} />
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {folderPanelMenu && !!(currentTab?.folderPath || "").trim() && (
        <div
          className="ss-ctx-menu"
          style={{ left: folderPanelMenu.x, top: folderPanelMenu.y }}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            onClick={() => {
              const root = (currentTab?.folderPath || "").trim();
              setFolderPanelMenu(null);
              if (root) void openPath(root);
            }}
          >
            打开原文件夹
          </button>
          <button
            type="button"
            onClick={() => {
              const cur = (tabBrowsePath || currentTab?.folderPath || "").trim();
              setFolderPanelMenu(null);
              if (cur) void openPath(cur);
            }}
          >
            打开当前目录
          </button>
          <button
            type="button"
            disabled={
              tabBrowsePath.replace(/[/\\]+$/, "").toLowerCase() ===
              (currentTab?.folderPath || "").replace(/[/\\]+$/, "").toLowerCase()
            }
            onClick={() => {
              const root = (currentTab?.folderPath || "").replace(/[/\\]+$/, "");
              const cur = tabBrowsePath.replace(/[/\\]+$/, "");
              setFolderPanelMenu(null);
              if (cur.toLowerCase() === root.toLowerCase()) return;
              const parent = cur.replace(/[/\\][^/\\]+$/, "");
              if (
                parent.length >= root.length &&
                parent.toLowerCase().startsWith(root.toLowerCase())
              ) {
                browseToDir(parent);
              } else {
                browseToDir(root);
              }
            }}
          >
            返回上级
          </button>
        </div>
      )}

      {itemMenu && (() => {
        const c = cfgRef.current;
        const list =
          itemMenu.scope === "home"
            ? c?.homeApps
            : c?.tabs.find((t) => t.id === itemMenu.scope)?.items;
        const it = list?.find((x) => x.id === itemMenu.id);
        const canReveal = !!it?.path && isFilesystemPath(it.path, it.kind);
        return (
          <div
            className="ss-ctx-menu"
            style={{ left: itemMenu.x, top: itemMenu.y }}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              type="button"
              onClick={() => {
                setItemMenu(null);
                if (it?.path) void openPath(it.path);
              }}
            >
              打开
            </button>
            {canReveal && (
              <button
                type="button"
                onClick={() => {
                  setItemMenu(null);
                  if (it?.path) void revealPath(it.path);
                }}
              >
                打开所在文件夹
              </button>
            )}
            <button
              type="button"
              className="danger"
              onClick={() => void removePinned(itemMenu.scope, itemMenu.id)}
            >
              移除
            </button>
          </div>
        );
      })()}

      <div className="ss-body-wrap">
      <main
        className={[
          "ss-body",
          panelDropMode === "ok" ? "is-file-drop-body" : "",
          panelDropMode === "blocked" ? "is-file-drop-body-blocked" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        data-ss-drop-zone
      >
        {evMsg && (
          <div className="ss-banner">
            <span>{evMsg}</span>
            <button type="button" onClick={() => void ensureEv()}>
              启动 Everything
            </button>
          </div>
        )}

        {searching ? (
          <>
            <div className="ss-search-cats">
              {SEARCH_TABS.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  className={`ss-search-cat ${searchCat === t.id ? "active" : ""}`}
                  onClick={() => setSearchCat(t.id)}
                >
                  {t.label}
                  {!searchBusy && catCount(t.id)}
                </button>
              ))}
            </div>
            {searchBusy ? (
              <div className="ss-empty ss-searching" role="status" aria-live="polite">
                <i className="ss-spinner ss-spinner-lg" aria-hidden />
                <span>正在搜索…</span>
              </div>
            ) : (
              renderSearchCat()
            )}
          </>
        ) : activeTab === "home" ? (
          <div className="ss-home">
            <section
              className={`ss-card apps${panelDropMode === "ok" && activeTab === "home" ? " is-file-drop" : ""}`}
              data-ss-drop="home"
            >
              <div className="ss-card-title">应用</div>
              <div className="ss-grid">
                {homeApps.length === 0 && (
                  <div className="ss-empty">拖入快捷方式 / exe，或点右下角 +</div>
                )}
                {renderPinnedItems(homeApps, "home")}
              </div>
            </section>
            <section className="ss-card tools">
              <div className="ss-card-title">工具</div>
              <div className="ss-grid tools">
                {TOOLS.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    className="ss-item"
                    onClick={() => void openSystem(t.id)}
                  >
                    <Icon name={t.name} path={t.path} />
                    <span className="ss-item-name">{t.name}</span>
                  </button>
                ))}
              </div>
            </section>
            <section className="ss-card recent">
              <div className="ss-card-title">最近文件</div>
              <div className="ss-grid recent">
                {recent.length === 0 && <div className="ss-empty">暂无最近文件</div>}
                {recent.map((r) => (
                  <button
                    key={r.path}
                    type="button"
                    className="ss-item ss-recent-row"
                    onClick={() => void openPath(r.path)}
                  >
                    <Icon png={r.iconPng} name={r.name} path={r.path} />
                    <div className="ss-recent-meta">
                      <div className="name">{r.name}</div>
                      <div className="path">{r.path}</div>
                    </div>
                  </button>
                ))}
              </div>
            </section>
          </div>
        ) : (
          <section
            className={[
              "ss-card ss-tab-page",
              panelDropMode === "ok" ? "is-file-drop" : "",
              panelDropMode === "blocked" ? "is-file-drop-blocked" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            data-ss-drop="panel"
            onContextMenu={
              (currentTab?.folderPath || "").trim()
                ? (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setTabMenu(null);
                    setAddMenu(null);
                    setFolderPanelMenu({ x: e.clientX, y: e.clientY });
                  }
                : undefined
            }
          >
            {!!(currentTab?.folderPath || "").trim() ? (
              <>
                <div
                  className="ss-grid ss-folder-grid"
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setTabMenu(null);
                    setAddMenu(null);
                    setFolderPanelMenu({ x: e.clientX, y: e.clientY });
                  }}
                >
                  {tabDirEntries.length === 0 && !tabDirLoading && (
                    <div className="ss-empty">文件夹为空 — 可从资源管理器拖入文件</div>
                  )}
                  {tabDirEntries.length === 0 && tabDirLoading && (
                    <div className="ss-empty ss-empty-muted">加载中…</div>
                  )}
                  {tabDirEntries.map((f) => (
                    <button
                      key={f.path}
                      type="button"
                      className="ss-item"
                      title={f.path}
                      onClick={() => {
                        if (f.isDir) {
                          browseToDir(f.path);
                        } else {
                          void openPath(f.path);
                        }
                      }}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setTabMenu(null);
                        setAddMenu(null);
                        setFolderPanelMenu({ x: e.clientX, y: e.clientY });
                      }}
                    >
                      <Icon png={f.iconPng} name={f.name} path={f.path} />
                      <span className="ss-item-name">{f.name}</span>
                    </button>
                  ))}
                </div>
              </>
            ) : (
              <div className="ss-grid">
                {(currentTab?.items || []).length === 0 && (
                  <div className="ss-empty">
                    拖入快捷方式 / exe / 文件夹，或点右下角 +
                  </div>
                )}
                {renderPinnedItems(currentTab?.items || [], currentTab?.id || activeTab)}
              </div>
            )}
          </section>
        )}

        {modalOpen && (
          <div className="ss-modal-mask" onClick={() => setModalOpen(false)}>
            <div className="ss-modal" onClick={(e) => e.stopPropagation()}>
              <div className="ss-modal-head">
                <strong>添加快捷方式</strong>
                <button type="button" className="ss-icon-btn" onClick={() => setModalOpen(false)}>
                  ×
                </button>
              </div>
              <div className="ss-modal-tabs">
                {(
                  [
                    ["app", "应用"],
                    ["url", "网址"],
                    ["file", "文件"],
                    ["folder", "文件夹"],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    className={`ss-modal-tab ${modalTab === id ? "active" : ""}`}
                    onClick={() => setModalTab(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <div className="ss-modal-body">
                {modalTab === "app" && (
                  <div className="ss-grid">
                    {apps.map((a) => {
                      const on = !!picked[a.id];
                      return (
                        <button
                          key={a.id}
                          type="button"
                          className={`ss-item ss-pick ${on ? "on" : ""}`}
                          onClick={() => {
                            setPicked((prev) => {
                              const next = { ...prev };
                              if (next[a.id]) delete next[a.id];
                              else next[a.id] = a;
                              return next;
                            });
                          }}
                        >
                          <Icon png={a.iconPng} name={a.name} path={a.target || a.path} />
                          <span className="ss-item-name">{a.name}</span>
                          {on && <span className="badge">+</span>}
                        </button>
                      );
                    })}
                  </div>
                )}
                {modalTab === "url" && (
                  <input
                    className="ss-search"
                    style={{ width: "100%", borderRadius: 8 }}
                    placeholder="https://..."
                    value={urlDraft}
                    onChange={(e) => setUrlDraft(e.target.value)}
                  />
                )}
                {(modalTab === "file" || modalTab === "folder") && (
                  <div className="ss-empty">
                    在地址框粘贴完整路径后点确定，或先切到「应用」勾选。
                    <input
                      style={{
                        display: "block",
                        width: "100%",
                        marginTop: 12,
                        height: 36,
                        padding: "0 10px",
                        borderRadius: 8,
                        border: "1px solid #e6e8ec",
                      }}
                      placeholder={modalTab === "folder" ? "C:\\path\\to\\folder" : "C:\\path\\to\\file"}
                      value={urlDraft}
                      onChange={(e) => setUrlDraft(e.target.value)}
                    />
                  </div>
                )}
              </div>
              <div className="ss-modal-foot">
                <button type="button" className="ss-btn" onClick={() => setModalOpen(false)}>
                  取消
                </button>
                <button
                  type="button"
                  className="ss-btn primary"
                  onClick={() => {
                    if ((modalTab === "file" || modalTab === "folder") && urlDraft.trim() && cfg) {
                      const p = urlDraft.trim();
                      const name = p.split(/[/\\]/).filter(Boolean).pop() || p;
                      const item: Shortcut = {
                        id: `${modalTab}-${Date.now()}`,
                        name,
                        path: p,
                        kind: modalTab,
                      };
                      const tabId = currentTab?.id || activeTab;
                      const tabs = cfg.tabs.map((t) =>
                        t.id === tabId ? { ...t, items: [...t.items, item] } : t,
                      );
                      const homeAppsNext =
                        activeTab === "home" || tabId === "home"
                          ? [...(cfg.homeApps || []), item]
                          : cfg.homeApps;
                      void persist({ ...cfg, tabs, homeApps: homeAppsNext }).then(() => {
                        setModalOpen(false);
                        setUrlDraft("");
                        showToast("已添加");
                      });
                      return;
                    }
                    void commitPicked();
                  }}
                >
                  确定
                </button>
              </div>
            </div>
          </div>
        )}

        {settingsOpen && cfg && (
          <div className="ss-modal-mask" onClick={() => setSettingsOpen(false)}>
            <div className="ss-settings" onClick={(e) => e.stopPropagation()}>
              <div className="ss-filter-head">
                <strong>搜搜设置</strong>
                <button
                  type="button"
                  className="ss-icon-btn"
                  onClick={() => setSettingsOpen(false)}
                  aria-label="关闭"
                >
                  ×
                </button>
              </div>

              <div className="ss-settings-body">
              <label className="ss-pref-row">
                <span>
                  <strong>启用搜搜</strong>
                  <small>关闭后不再响应双击 Ctrl</small>
                </span>
                <button
                  type="button"
                  className={`ss-toggle${cfg.enabled ? " on" : ""}`}
                  role="switch"
                  aria-checked={cfg.enabled}
                  onClick={() => void patchPrefs({ enabled: !cfg.enabled })}
                >
                  <span />
                </button>
              </label>

              <label className="ss-pref-row">
                <span>
                  <strong>双击 Ctrl 热键</strong>
                  <small>连续两次松开 Ctrl 唤起/隐藏</small>
                </span>
                <button
                  type="button"
                  className={`ss-toggle${cfg.hotkeyEnabled ? " on" : ""}`}
                  role="switch"
                  aria-checked={cfg.hotkeyEnabled}
                  disabled={!cfg.enabled}
                  onClick={() => void patchPrefs({ hotkeyEnabled: !cfg.hotkeyEnabled })}
                >
                  <span />
                </button>
              </label>

              <label className="ss-pref-row">
                <span>
                  <strong>双击间隔 (ms)</strong>
                  <small>两次 Ctrl 松开的最大间隔</small>
                </span>
                <input
                  className="ss-pref-input"
                  type="number"
                  min={100}
                  max={2000}
                  value={cfg.doubleCtrlMs}
                  onChange={(e) =>
                    setCfg({ ...cfg, doubleCtrlMs: Number(e.target.value) || 350 })
                  }
                  onBlur={() => void patchPrefs({ doubleCtrlMs: cfg.doubleCtrlMs })}
                />
              </label>

              <label className="ss-pref-row">
                <span>
                  <strong>悬停切换标签</strong>
                  <small>鼠标移到标签即可切换，无需点击</small>
                </span>
                <button
                  type="button"
                  className={`ss-toggle${cfg.hoverSwitchTabs !== false ? " on" : ""}`}
                  role="switch"
                  aria-checked={cfg.hoverSwitchTabs !== false}
                  onClick={() =>
                    void patchPrefs({ hoverSwitchTabs: cfg.hoverSwitchTabs === false })
                  }
                >
                  <span />
                </button>
              </label>

              <label className="ss-pref-row">
                <span>
                  <strong>打开后关闭面板</strong>
                  <small>点击应用/文件后自动隐藏搜搜</small>
                </span>
                <button
                  type="button"
                  className={`ss-toggle${cfg.closeAfterOpen !== false ? " on" : ""}`}
                  role="switch"
                  aria-checked={cfg.closeAfterOpen !== false}
                  onClick={() =>
                    void patchPrefs({ closeAfterOpen: cfg.closeAfterOpen === false })
                  }
                >
                  <span />
                </button>
              </label>

              <label className="ss-pref-row stack">
                <span>
                  <strong>Everything.exe</strong>
                </span>
                <input
                  className="ss-pref-input wide"
                  value={cfg.everythingExe}
                  onChange={(e) => setCfg({ ...cfg, everythingExe: e.target.value })}
                  onBlur={() => void patchPrefs({ everythingExe: cfg.everythingExe })}
                />
              </label>

              <label className="ss-pref-row stack">
                <span>
                  <strong>es.exe</strong>
                </span>
                <input
                  className="ss-pref-input wide"
                  value={cfg.esExe}
                  onChange={(e) => setCfg({ ...cfg, esExe: e.target.value })}
                  onBlur={() => void patchPrefs({ esExe: cfg.esExe })}
                />
              </label>

              <label className="ss-pref-row">
                <span>
                  <strong>图标缓存</strong>
                  <small>
                    {iconCacheStats
                      ? `${iconCacheStats.entries} 个 · ${formatSize(iconCacheStats.bytes)}`
                      : "加载中…"}
                  </small>
                </span>
                <button type="button" className="ss-btn" onClick={() => void clearIconCache()}>
                  清除缓存
                </button>
              </label>
              {evMsg && <p className="ss-settings-hint">{evMsg}</p>}
              </div>

              <div className="ss-filter-foot">
                <button type="button" className="ss-btn" onClick={() => void ensureEv()}>
                  启动 / 检测 Everything
                </button>
                <button type="button" className="ss-btn primary" onClick={() => setSettingsOpen(false)}>
                  完成
                </button>
              </div>
            </div>
          </div>
        )}

        {filterOpen && (
          <div className="ss-modal-mask" onClick={() => setFilterOpen(false)}>
            <div className="ss-filter" onClick={(e) => e.stopPropagation()}>
              <div className="ss-filter-head">
                <strong>搜索结果筛选</strong>
                <div className="ss-filter-head-actions">
                  <label className="ss-switch">
                    <span>启用</span>
                    <button
                      type="button"
                      className={`ss-toggle${filter.enabled ? " on" : ""}`}
                      role="switch"
                      aria-checked={filter.enabled}
                      onClick={() => setFilter((f) => ({ ...f, enabled: !f.enabled }))}
                    >
                      <span />
                    </button>
                  </label>
                  <button
                    type="button"
                    className="ss-link"
                    onClick={() => setFilter(defaultFilter())}
                  >
                    重置
                  </button>
                  <button
                    type="button"
                    className="ss-icon-btn"
                    onClick={() => setFilterOpen(false)}
                    aria-label="关闭"
                  >
                    ×
                  </button>
                </div>
              </div>

              <div className="ss-filter-row">
                <div className="ss-filter-label">名称:</div>
                <label className="ss-check">
                  <input
                    type="checkbox"
                    checked={filter.wholeWord}
                    onChange={(e) => setFilter((f) => ({ ...f, wholeWord: e.target.checked }))}
                  />
                  全字匹配
                </label>
              </div>

              <div className="ss-filter-row">
                <div className="ss-filter-label">路径:</div>
                <div className="ss-filter-col">
                  <div className="ss-path-row">
                    <input
                      value={filter.path}
                      onChange={(e) => setFilter((f) => ({ ...f, path: e.target.value }))}
                      placeholder="C:\"
                    />
                    <button
                      type="button"
                      className="ss-link"
                      onClick={() => {
                        void invoke<string | null>("sousou_pick_folder").then((p) => {
                          if (p) setFilter((f) => ({ ...f, path: p }));
                        });
                      }}
                    >
                      📁 选择文件夹路径
                    </button>
                  </div>
                  <label className="ss-check">
                    <input
                      type="checkbox"
                      checked={filter.includeSubfolders}
                      onChange={(e) =>
                        setFilter((f) => ({ ...f, includeSubfolders: e.target.checked }))
                      }
                    />
                    包含子文件夹
                  </label>
                </div>
              </div>

              <div className="ss-filter-row">
                <div className="ss-filter-label">后缀名:</div>
                <div className="ss-filter-col">
                  <div className="ss-chip-row">
                    {["doc", "ppt", "xls", "xlsx", "pdf"].map((ext) => (
                      <label key={ext} className="ss-check">
                        <input
                          type="checkbox"
                          checked={filter.extPresets.includes(ext)}
                          onChange={() => toggleExt(ext)}
                        />
                        {ext}
                      </label>
                    ))}
                  </div>
                  <input
                    value={filter.extCustom}
                    onChange={(e) => setFilter((f) => ({ ...f, extCustom: e.target.value }))}
                    placeholder="输入后缀名多个使用英文逗号隔开"
                  />
                </div>
              </div>

              <div className="ss-filter-row">
                <div className="ss-filter-label">修改时间:</div>
                <div className="ss-chip-row">
                  {(
                    [
                      ["today", "今天"],
                      ["yesterday", "昨天"],
                      ["last3", "近3天"],
                      ["last7", "近7天"],
                      ["last30", "近30天"],
                    ] as const
                  ).map(([id, label]) => (
                    <label key={id} className="ss-check">
                      <input
                        type="radio"
                        name="ss-modified"
                        checked={filter.modified === id}
                        onChange={() =>
                          setFilter((f) => ({
                            ...f,
                            modified: f.modified === id ? "" : id,
                          }))
                        }
                      />
                      {label}
                    </label>
                  ))}
                </div>
              </div>

              <div className="ss-filter-row">
                <div className="ss-filter-label">大小:</div>
                <div className="ss-filter-col">
                  <div className="ss-chip-row">
                    {(
                      [
                        ["tiny", "极小(0-20k)"],
                        ["medium", "中等(1-128M)"],
                        ["large", "较大(128M以上)"],
                      ] as const
                    ).map(([id, label]) => (
                      <label key={id} className="ss-check">
                        <input
                          type="radio"
                          name="ss-size"
                          checked={filter.sizePreset === id}
                          onChange={() =>
                            setFilter((f) => ({
                              ...f,
                              sizePreset: f.sizePreset === id ? "" : id,
                            }))
                          }
                        />
                        {label}
                      </label>
                    ))}
                  </div>
                  <div className="ss-size-row">
                    <input
                      value={filter.sizeMin}
                      onChange={(e) =>
                        setFilter((f) => ({ ...f, sizeMin: e.target.value, sizePreset: "" }))
                      }
                    />
                    <select
                      value={filter.sizeMinUnit}
                      onChange={(e) => setFilter((f) => ({ ...f, sizeMinUnit: e.target.value }))}
                    >
                      <option>KB</option>
                      <option>MB</option>
                      <option>GB</option>
                    </select>
                    <span>-</span>
                    <input
                      value={filter.sizeMax}
                      onChange={(e) =>
                        setFilter((f) => ({ ...f, sizeMax: e.target.value, sizePreset: "" }))
                      }
                    />
                    <select
                      value={filter.sizeMaxUnit}
                      onChange={(e) => setFilter((f) => ({ ...f, sizeMaxUnit: e.target.value }))}
                    >
                      <option>KB</option>
                      <option>MB</option>
                      <option>GB</option>
                    </select>
                  </div>
                </div>
              </div>

              <div className="ss-filter-foot">
                <button type="button" className="ss-btn" onClick={() => setFilterOpen(false)}>
                  取消
                </button>
                <button
                  type="button"
                  className="ss-btn primary"
                  onClick={() => {
                    void saveFilter(filter).then(() => setFilterOpen(false));
                  }}
                >
                  确定
                </button>
              </div>
            </div>
          </div>
        )}

      </main>

      {!searching && !(currentTab?.folderPath || "").trim() && (
        <button
          type="button"
          className="ss-fab"
          title="添加"
          aria-label="添加快捷方式"
          onClick={() => {
            setPicked({});
            setModalOpen(true);
            void loadPickerApps();
          }}
        >
          +
        </button>
      )}
      {toast && <div className="ss-toast">{toast}</div>}
      </div>

      <footer className="ss-footer">
        <div style={{ display: "flex", gap: 4 }}>
          <button type="button" className="ss-footer-btn" onClick={() => void openSystem("settings")}>
            ⚙ 系统设置
          </button>
          <button type="button" className="ss-footer-btn" onClick={() => void openSystem("control")}>
            ▦ 控制面板
          </button>
        </div>
        <button type="button" className="ss-footer-btn" onClick={() => void openSystem("computer")}>
          🖥 我的电脑
        </button>
      </footer>
    </div>
  );
}
