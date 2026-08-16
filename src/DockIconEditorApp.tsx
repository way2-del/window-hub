import { Component, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  normalizeGlassKind,
  subscribeSystemDark,
  syncGlassCss,
  type GlassPrefs,
} from "./glassPrefs";
import {
  DockStartIcon,
  DockTrashIcon,
  DOCK_AUTO_PLATE_BG,
  DOCK_START_BG,
  DOCK_TRASH_BG,
} from "./dockIcons";
import { plateColorFromPngBase64, peekCachedPlateColor } from "./dockIconBg";
import "./settings.css";
import "./dockIconEditor.css";

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
  iconBg?: string;
};

type DockPrefs = {
  enabled: boolean;
  displayMode: string;
  hideSystemTaskbar: boolean;
  items: DockItem[];
  hotkey: string;
  hiddenItemIds?: string[];
  hoverWindowPreview?: boolean;
};

/** Matches Rust `default_icon_scale` (0.9 → 90%). */
const DEFAULT_SCALE_PCT = 90;

const BG_PRESETS: Array<{ id: string; label: string; value: string }> = [
  { id: "auto", label: "自动", value: "" },
  { id: "plate", label: "浅灰", value: DOCK_AUTO_PLATE_BG },
  { id: "white", label: "白", value: "#ffffff" },
  { id: "black", label: "黑", value: "#1c1c1e" },
  { id: "blue", label: "蓝", value: DOCK_START_BG },
  { id: "none", label: "透明", value: "transparent" },
];

function itemTitle(it: DockItem): string {
  if (it.kind === "startmenu") return "开始";
  if (it.kind === "trash") return "回收站";
  return (it.label || it.matchExe || it.id).trim() || it.id;
}

function itemSubtitle(it: DockItem): string {
  if (it.kind === "startmenu") return "startmenu";
  if (it.kind === "trash") return "trash";
  const exe = (it.matchExe || "").trim();
  if (exe) return exe;
  const path = (it.launchPath || "").trim();
  if (!path) return "—";
  const slash = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return slash >= 0 ? path.slice(slash + 1) : path;
}

function scalePctFromItem(it: DockItem): number {
  const s = typeof it.iconScale === "number" && it.iconScale > 0 ? it.iconScale : 0.9;
  return Math.round(s * 100);
}

function resolveStaticPlateBg(item: DockItem, draftBg?: string): string | null {
  const raw = (draftBg ?? item.iconBg ?? "").trim();
  if (raw === "transparent" || raw === "none") return "transparent";
  if (raw) return raw;
  if (item.kind === "startmenu" && !item.iconPng) return DOCK_START_BG;
  if (item.kind === "trash" && !item.iconPng) return DOCK_TRASH_BG;
  return null; // auto → sample from PNG
}

function ItemGlyph({ item }: { item: DockItem }) {
  if (item.kind === "startmenu" && !item.iconPng) return <DockStartIcon />;
  if (item.kind === "trash" && !item.iconPng) return <DockTrashIcon />;
  if (item.iconPng) {
    return <img src={`data:image/png;base64,${item.iconPng}`} alt="" draggable={false} />;
  }
  return <span className="die-fallback-letter">{itemTitle(item).charAt(0)}</span>;
}

function useAutoPlateBg(item: DockItem, draftBg?: string): string {
  const staticBg = resolveStaticPlateBg(item, draftBg);
  const png = (item.iconPng || "").trim();
  const [auto, setAuto] = useState<string>(
    () => (staticBg != null ? staticBg : peekCachedPlateColor(png) || DOCK_AUTO_PLATE_BG),
  );

  useEffect(() => {
    if (staticBg != null) {
      setAuto(staticBg);
      return;
    }
    if (!png) {
      setAuto(DOCK_AUTO_PLATE_BG);
      return;
    }
    const cached = peekCachedPlateColor(png);
    if (cached) {
      setAuto(cached);
      return;
    }
    let alive = true;
    void plateColorFromPngBase64(png).then((c) => {
      if (alive) setAuto(c);
    });
    return () => {
      alive = false;
    };
  }, [staticBg, png]);

  return auto;
}

function ItemThumb({
  item,
  draftBg,
  scalePct,
  offsetX,
  offsetY,
}: {
  item: DockItem;
  /** Live draft from the editor when this row is selected. */
  draftBg?: string;
  scalePct?: number;
  offsetX?: number;
  offsetY?: number;
}) {
  const bg = useAutoPlateBg(item, draftBg);
  const scale = Math.min(2, Math.max(0.5, (scalePct ?? scalePctFromItem(item)) / 100));
  const ox = offsetX ?? Math.round(item.iconOffsetX ?? 0);
  const oy = offsetY ?? Math.round(item.iconOffsetY ?? 0);
  // Thumb is 22px; preview uses ×2 on offsets — keep proportional (~22/64 of preview feel).
  const oxPx = ox * (22 / 64);
  const oyPx = oy * (22 / 64);
  return (
    <span className="die-nav-thumb" style={{ background: bg }}>
      <span
        className="die-nav-glyph"
        style={{
          transform: `translate(${oxPx}px, ${oyPx}px) scale(${scale})`,
        }}
      >
        <ItemGlyph item={item} />
      </span>
    </span>
  );
}

class EditorErrorBoundary extends Component<
  { children: ReactNode },
  { error: string | null }
> {
  state = { error: null as string | null };
  static getDerivedStateFromError(err: unknown) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
  render() {
    if (this.state.error) {
      return (
        <div className="die-error">
          <strong>修改图标界面加载失败</strong>
          <pre>{this.state.error}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

function DockIconEditorInner() {
  const [prefs, setPrefs] = useState<DockPrefs | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState("");
  const [draftScale, setDraftScale] = useState(DEFAULT_SCALE_PCT);
  const [draftOx, setDraftOx] = useState(0);
  const [draftOy, setDraftOy] = useState(0);
  const [draftBg, setDraftBg] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const editable = useMemo(
    () => (prefs?.items ?? []).filter((it) => it.kind !== "separator" && !it.id.startsWith("running:")),
    [prefs],
  );

  const selected = useMemo(
    () => editable.find((it) => it.id === selectedId) ?? editable[0] ?? null,
    [editable, selectedId],
  );

  const applyItemDrafts = (hit: DockItem) => {
    setSelectedId(hit.id);
    setDraftScale(scalePctFromItem(hit));
    setDraftOx(Math.round(hit.iconOffsetX ?? 0));
    setDraftOy(Math.round(hit.iconOffsetY ?? 0));
    setDraftBg((hit.iconBg ?? "").trim());
  };

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    const applyGlass = async () => {
      try {
        const material = await invoke<GlassPrefs>("get_material_prefs");
        await syncGlassCss({
          ...material,
          kind: normalizeGlassKind(material.kind),
        });
      } catch {
        await syncGlassCss({ kind: "mica-alt", dark: true });
      }
      await invoke("apply_window_effect", {}).catch(() => undefined);
    };

    const load = async (focusId?: string) => {
      try {
        const p = await invoke<DockPrefs>("get_dock_prefs");
        if (cancelled) return;
        setPrefs(p);
        setLoadError(null);
        const list = p.items.filter(
          (it) => it.kind !== "separator" && !it.id.startsWith("running:"),
        );
        const want = (
          focusId ||
          (typeof window.__WH_DOCK_ICON_EDITOR_FOCUS__ === "string"
            ? window.__WH_DOCK_ICON_EDITOR_FOCUS__
            : "") ||
          selectedId ||
          list[0]?.id ||
          ""
        ).trim();
        const hit = list.find((it) => it.id === want) ?? list[0] ?? null;
        if (hit) applyItemDrafts(hit);
      } catch (e) {
        console.error(e);
        if (!cancelled) setLoadError(String(e));
      }
    };

    void applyGlass();
    void load();
    const t1 = window.setTimeout(() => void applyGlass(), 120);
    const t2 = window.setTimeout(() => void applyGlass(), 400);

    const unDark = subscribeSystemDark(() => {
      void applyGlass();
    });
    void listen<GlassPrefs>("material-prefs", () => {
      void applyGlass();
    }).then((u) => unsubs.push(u));
    void listen<DockPrefs>("dock-prefs", (ev) => {
      if (cancelled) return;
      // Always take host payload (includes freshly materialized iconPng).
      setPrefs(ev.payload);
    }).then((u) => unsubs.push(u));
    void listen<string>("dock-icon-editor-focus", (ev) => {
      const id = String(ev.payload || "").trim();
      if (id) void load(id);
    }).then((u) => unsubs.push(u));

    return () => {
      cancelled = true;
      window.clearTimeout(t1);
      window.clearTimeout(t2);
      unDark();
      for (const u of unsubs) u();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!selected) return;
    setDraftScale(scalePctFromItem(selected));
    setDraftOx(Math.round(selected.iconOffsetX ?? 0));
    setDraftOy(Math.round(selected.iconOffsetY ?? 0));
    setDraftBg((selected.iconBg ?? "").trim());
    setMsg(null);
  }, [selected?.id]);

  async function persist(nextItems: DockItem[]): Promise<DockPrefs | null> {
    if (!prefs) return null;
    setBusy(true);
    setMsg(null);
    try {
      const saved = await invoke<DockPrefs>("set_dock_prefs", {
        prefs: { ...prefs, items: nextItems },
      });
      setPrefs(saved);
      setMsg("已保存");
      return saved;
    } catch (e) {
      console.error(e);
      setMsg(String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }

  async function onRestore() {
    if (!prefs || !selected) return;
    const next = prefs.items.map((it) =>
      it.id === selected.id
        ? {
            ...it,
            iconPath: "",
            iconScale: 0.9,
            iconOffsetX: 0,
            iconOffsetY: 0,
            iconBg: "",
            iconPng: null,
          }
        : it,
    );
    setDraftScale(DEFAULT_SCALE_PCT);
    setDraftOx(0);
    setDraftOy(0);
    setDraftBg("");
    await persist(next);
  }

  async function onPick() {
    if (!prefs || !selected) return;
    try {
      const path = await invoke<string | null>("pick_dock_icon_file");
      if (!path) return;
      setBusy(true);
      setMsg(null);
      // Materialize into `%APPDATA%\window-hub\dock-icons\{id}.png`.
      const cached = await invoke<string>("dock_cache_icon", {
        itemId: selected.id,
        sourcePath: path,
      });
      const nextItems = prefs.items.map((it) =>
        it.id === selected.id ? { ...it, iconPath: cached, iconPng: null } : it,
      );
      const saved = await invoke<DockPrefs>("set_dock_prefs", {
        prefs: { ...prefs, items: nextItems },
      });
      // Prefer host-resolved PNG so the left list updates immediately.
      let withPng = saved;
      const hit = saved.items.find((it) => it.id === selected.id);
      if (hit && !hit.iconPng) {
        withPng = await invoke<DockPrefs>("get_dock_prefs");
      }
      const fresh = withPng.items.find((it) => it.id === selected.id);
      // Auto plate = island-notify dominant color (keep iconBg empty = 自动).
      if (fresh?.iconPng) {
        await plateColorFromPngBase64(fresh.iconPng);
      }
      setPrefs(withPng);
      setDraftBg("");
      setMsg("已保存");
    } catch (e) {
      console.error(e);
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onConfirm() {
    if (!prefs || !selected) return;
    const scale = Math.min(200, Math.max(50, draftScale)) / 100;
    const ox = Math.min(24, Math.max(-24, draftOx));
    const oy = Math.min(24, Math.max(-24, draftOy));
    const bg = draftBg.trim();
    const next = prefs.items.map((it) =>
      it.id === selected.id
        ? { ...it, iconScale: scale, iconOffsetX: ox, iconOffsetY: oy, iconBg: bg }
        : it,
    );
    await persist(next);
  }

  const previewScale = Math.min(200, Math.max(50, draftScale)) / 100;
  const plateBg = useAutoPlateBg(
    selected ?? {
      id: "",
      kind: "",
      label: "",
      matchExe: "",
      launchPath: "",
      realPath: "",
      virtualPath: "",
      iconPath: "",
      uwp: false,
      iconPng: null,
      iconBg: "",
    },
    selected ? draftBg : "",
  );
  const colorPickerValue =
    draftBg && draftBg !== "transparent" && draftBg.startsWith("#")
      ? draftBg.length === 4
        ? `#${draftBg[1]}${draftBg[1]}${draftBg[2]}${draftBg[2]}${draftBg[3]}${draftBg[3]}`
        : draftBg.slice(0, 7)
      : plateBg.startsWith("#")
        ? plateBg.slice(0, 7)
        : "#f2f2f7";

  if (loadError) {
    return (
      <div className="die-error">
        <strong>无法读取 Dock 配置</strong>
        <pre>{loadError}</pre>
      </div>
    );
  }

  if (!prefs) {
    return (
      <div className="die-loading" role="status">
        正在加载…
      </div>
    );
  }

  return (
    <div className="settings-shell die-shell">
      <aside className="settings-side die-side">
        <div className="die-side-title">Dock</div>
        <nav className="settings-nav die-nav">
          {editable.length === 0 ? (
            <p className="die-nav-empty">暂无图标</p>
          ) : (
            editable.map((it) => {
              const active = selected?.id === it.id;
              return (
                <button
                  key={it.id}
                  type="button"
                  className={`settings-nav-item die-nav-item${active ? " is-active" : ""}`}
                  onClick={() => setSelectedId(it.id)}
                >
                  <span className="die-nav-thumb-wrap" aria-hidden>
                    <ItemThumb
                      item={it}
                      draftBg={active ? draftBg : undefined}
                      scalePct={active ? draftScale : undefined}
                      offsetX={active ? draftOx : undefined}
                      offsetY={active ? draftOy : undefined}
                    />
                  </span>
                  <span className="die-nav-text">
                    <span className="die-nav-label">{itemTitle(it)}</span>
                    <span className="die-nav-sub">{itemSubtitle(it)}</span>
                  </span>
                </button>
              );
            })
          )}
        </nav>
      </aside>

      <main className="settings-main die-main">
        {!selected ? (
          <div className="die-empty">
            <p>暂无 Dock 图标可编辑。请先在设置里导入或启用 Dock。</p>
          </div>
        ) : (
          <>
            <header className="die-head">
              <div className="die-head-text">
                <h1>{itemTitle(selected)}</h1>
                <p>{itemSubtitle(selected)}</p>
              </div>
            </header>

            <div className="die-body">
              <section className="die-preview" aria-label="图标预览">
                <div className="die-preview-plate" style={{ background: plateBg }}>
                  <div
                    className="die-preview-glyph"
                    style={{
                      transform: `translate(${draftOx * 2}px, ${draftOy * 2}px) scale(${previewScale})`,
                    }}
                  >
                    <ItemGlyph item={selected} />
                  </div>
                </div>
              </section>

              <section className="die-panel">
                <h2>图标</h2>
                <div className="die-actions">
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={busy}
                    onClick={() => void onRestore()}
                  >
                    还原
                  </button>
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={busy}
                    onClick={() => void onPick()}
                  >
                    选择图片
                  </button>
                </div>

                <div className="die-bg-block">
                  <strong>背景色</strong>
                  <p className="die-bg-hint">
                    「自动」复用消息岛描边取色：从图标主色生成底板
                  </p>
                  <div className="die-bg-presets" role="list">
                    {BG_PRESETS.map((p) => {
                      const active =
                        p.value === ""
                          ? draftBg === ""
                          : draftBg.toLowerCase() === p.value.toLowerCase();
                      return (
                        <button
                          key={p.id}
                          type="button"
                          className={`die-bg-swatch${active ? " is-active" : ""}`}
                          title={p.label}
                          style={{
                            background:
                              p.value === ""
                                ? plateBg.startsWith("#")
                                  ? plateBg
                                  : "conic-gradient(from 90deg, #f2f2f7, #0078D4, #1c1c1e, #f2f2f7)"
                                : p.value === "transparent"
                                  ? "repeating-conic-gradient(#666 0% 25%, #333 0% 50%) 50% / 10px 10px"
                                  : p.value,
                          }}
                          onClick={() => setDraftBg(p.value)}
                        >
                          <span>{p.label}</span>
                        </button>
                      );
                    })}
                    <label className="die-bg-custom" title="自定义颜色">
                      <input
                        type="color"
                        value={colorPickerValue}
                        onChange={(e) => setDraftBg(e.target.value)}
                      />
                      <span>自定义</span>
                    </label>
                  </div>
                </div>

                <label className="material-slider">
                  <strong>图标缩放 {draftScale}%</strong>
                  <input
                    type="range"
                    min={50}
                    max={200}
                    step={1}
                    value={draftScale}
                    onChange={(e) => setDraftScale(Number(e.target.value))}
                  />
                </label>
                <label className="material-slider">
                  <strong>横向偏移 {draftOx}px</strong>
                  <input
                    type="range"
                    min={-24}
                    max={24}
                    step={1}
                    value={draftOx}
                    onChange={(e) => setDraftOx(Number(e.target.value))}
                  />
                </label>
                <label className="material-slider">
                  <strong>纵向偏移 {draftOy}px</strong>
                  <input
                    type="range"
                    min={-24}
                    max={24}
                    step={1}
                    value={draftOy}
                    onChange={(e) => setDraftOy(Number(e.target.value))}
                  />
                </label>

                <div className="die-footer">
                  {msg ? <span className="die-msg">{msg}</span> : <span />}
                  <button
                    type="button"
                    className="settings-primary-btn"
                    disabled={busy}
                    onClick={() => void onConfirm()}
                  >
                    确定
                  </button>
                </div>
              </section>
            </div>
          </>
        )}
      </main>
    </div>
  );
}

export default function DockIconEditorApp() {
  return (
    <EditorErrorBoundary>
      <DockIconEditorInner />
    </EditorErrorBoundary>
  );
}
