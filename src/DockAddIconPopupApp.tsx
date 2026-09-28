import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { fitPopupToContent } from "./popupFit";
import ChromePopupShell, {
  CHROME_POPUP_SHELL_SELECTOR,
} from "./features/chromePopup/ChromePopupShell";
import { DockStartIcon, DockTrashIcon } from "./dockIcons";
import "./dockAddIconPopup.css";

declare global {
  interface Window {
    __WH_DOCK_ADD_ICON_AFTER_ITEM_ID__?: string | null;
    __WH_DOCK_ADD_ICON_PIN_BOTTOM__?: number | null;
  }
}

type OpenPayload = {
  afterItemId?: string | null;
  pinBottom?: number | null;
};

type Preset = {
  id: string;
  label: string;
  present: boolean;
};

const POPUP_W = 280;

let afterItemIdCached: string | null | undefined;
let pinBottomCached: number | null | undefined;

function resetCaches() {
  afterItemIdCached = undefined;
  pinBottomCached = undefined;
}

function applyPayload(payload?: OpenPayload | null) {
  if (!payload) return;
  if (payload.afterItemId !== undefined) {
    const id = (payload.afterItemId ?? "").trim() || null;
    afterItemIdCached = id;
    window.__WH_DOCK_ADD_ICON_AFTER_ITEM_ID__ = id;
  }
  if (payload.pinBottom !== undefined) {
    const n = payload.pinBottom;
    pinBottomCached = typeof n === "number" && Number.isFinite(n) ? n : null;
    window.__WH_DOCK_ADD_ICON_PIN_BOTTOM__ = pinBottomCached;
  }
}

function readAfterItemId(): string | null {
  if (afterItemIdCached !== undefined) return afterItemIdCached;
  const fromWin = window.__WH_DOCK_ADD_ICON_AFTER_ITEM_ID__;
  if (typeof fromWin === "string" && fromWin.trim()) {
    afterItemIdCached = fromWin.trim();
    return afterItemIdCached;
  }
  if (fromWin === null) {
    afterItemIdCached = null;
    return null;
  }
  afterItemIdCached = null;
  return null;
}

function readPinBottom(): number | null {
  if (pinBottomCached !== undefined) return pinBottomCached;
  const fromWin = window.__WH_DOCK_ADD_ICON_PIN_BOTTOM__;
  if (typeof fromWin === "number" && Number.isFinite(fromWin)) {
    pinBottomCached = fromWin;
    return fromWin;
  }
  pinBottomCached = null;
  return null;
}

async function closeSelf() {
  try {
    await invoke("close_dock_add_icon_popup");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

async function syncGlass() {
  try {
    const prefs = await invoke<GlassPrefs>("get_material_prefs");
    await syncGlassCss({
      kind: "mica-alt",
      dark: prefs.dark ?? null,
      acrylicAlpha: prefs.acrylicAlpha,
    });
  } catch {
    await syncGlassCss({ kind: "mica-alt", dark: true });
  }
  await invoke("apply_window_effect", {}).catch(() => undefined);
}

async function revealFitted() {
  const pinBottom = readPinBottom();
  await fitPopupToContent({
    width: POPUP_W,
    selector: CHROME_POPUP_SHELL_SELECTOR,
    minHeight: 120,
    maxHeight: 520,
    pinBottom,
  });
  const win = getCurrentWindow();
  await win.show();
  await win.setFocus();
  await invoke("apply_window_effect", {}).catch(() => undefined);
}

function PresetGlyph({ id }: { id: string }) {
  if (id === "startmenu") return <DockStartIcon />;
  if (id === "trash") return <DockTrashIcon />;
  if (id === "explorer") {
    return (
      <svg viewBox="0 0 32 32" width="100%" height="100%" aria-hidden>
        <rect x="4" y="8" width="24" height="18" rx="2.5" fill="#F0C040" />
        <path d="M4 12h24v3H4z" fill="#E0A820" />
        <rect x="7" y="16" width="18" height="7" rx="1" fill="#FFF8E0" opacity="0.9" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 32 32" width="100%" height="100%" aria-hidden>
      <rect x="6" y="6" width="20" height="20" rx="3" fill="#0078D4" />
      <circle cx="16" cy="16" r="5.5" fill="none" stroke="#fff" strokeWidth="1.8" />
      <path
        d="M16 8.5v2.2M16 21.3v2.2M8.5 16h2.2M21.3 16h2.2"
        stroke="#fff"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

async function pinPaths(paths: string[], useIconMask: boolean) {
  const cleaned = paths.map((p) => p.trim()).filter(Boolean);
  if (!cleaned.length) return;
  await invoke("dock_pin_paths", {
    paths: cleaned,
    afterItemId: readAfterItemId(),
    useIconMask,
  });
  await closeSelf();
}

export default function DockAddIconPopupApp() {
  const [presets, setPresets] = useState<Preset[] | null>(null);
  const [entered, setEntered] = useState(false);
  const [useIconMask, setUseIconMask] = useState(true);
  const [dropHover, setDropHover] = useState(false);
  const [busy, setBusy] = useState(false);
  const revealGen = useRef(0);
  const reuseArmedRef = useRef(false);
  const useIconMaskRef = useRef(true);
  const busyRef = useRef(false);

  useEffect(() => {
    useIconMaskRef.current = useIconMask;
  }, [useIconMask]);

  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    const start = async () => {
      await syncGlass();
      if (cancelled) return;
      let list: Preset[] = [];
      try {
        list = await invoke<Preset[]>("list_dock_system_icon_presets");
      } catch (e) {
        console.error(e);
        list = [
          { id: "explorer", label: "资源管理器", present: false },
          { id: "startmenu", label: "开始菜单", present: false },
          { id: "trash", label: "废纸篓", present: false },
          { id: "controlpanel", label: "控制面板", present: false },
        ];
      }
      if (cancelled) return;
      setEntered(false);
      setPresets(list);
    };

    void start();

    const retryA = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 120);
    const retryB = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 350);

    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({
        kind: "mica-alt",
        dark: ev.payload.dark ?? null,
        acrylicAlpha: ev.payload.acrylicAlpha,
      });
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeSystemDark(() => {
        void (async () => {
          try {
            const prefs = await invoke<GlassPrefs>("get_material_prefs");
            if (prefs.dark != null) return;
            await syncGlassCss({ kind: "mica-alt", dark: null });
            await invoke("apply_window_effect", {}).catch(() => undefined);
          } catch {
            /* noop */
          }
        })();
      }),
    );

    void listen<OpenPayload>("dock-add-icon-popup-opened", (ev) => {
      if (!reuseArmedRef.current) {
        applyPayload(ev.payload);
        return;
      }
      resetCaches();
      applyPayload(ev.payload);
      void start();
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void getCurrentWindow()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") {
          setDropHover(true);
        } else if (p.type === "leave") {
          setDropHover(false);
        } else if (p.type === "drop") {
          setDropHover(false);
          const paths = p.paths ?? [];
          if (!paths.length || busyRef.current) return;
          setBusy(true);
          void pinPaths(paths, useIconMaskRef.current)
            .catch((e) => console.error("[DockAddIconPopup] drop", e))
            .finally(() => setBusy(false));
        }
      })
      .then((fn) => {
        if (!cancelled) unsubs.push(fn);
        else fn();
      });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void closeSelf();
    };
    document.addEventListener("keydown", onKey);

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      document.removeEventListener("keydown", onKey);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  useLayoutEffect(() => {
    if (!presets) return;
    let cancelled = false;
    const gen = ++revealGen.current;
    void (async () => {
      try {
        await revealFitted();
        if (cancelled || gen !== revealGen.current) return;
        reuseArmedRef.current = true;
        setEntered(true);
      } catch (e) {
        console.error("[DockAddIconPopup]", e);
        try {
          await getCurrentWindow().show();
          reuseArmedRef.current = true;
          if (!cancelled) setEntered(true);
        } catch {
          /* noop */
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [presets]);

  async function pick(preset: Preset) {
    if (preset.present && (preset.id === "startmenu" || preset.id === "trash")) {
      return;
    }
    if (busy) return;
    setBusy(true);
    try {
      await invoke("dock_add_system_icon", {
        preset: preset.id,
        afterItemId: readAfterItemId(),
      });
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
      await closeSelf();
    }
  }

  async function browseFiles() {
    if (busy) return;
    setBusy(true);
    try {
      const paths = await invoke<string[]>("pick_dock_pin_files");
      if (!paths?.length) return;
      await pinPaths(paths, useIconMask);
    } catch (e) {
      console.error("[DockAddIconPopup] browse", e);
    } finally {
      setBusy(false);
    }
  }

  if (!presets) {
    return <ChromePopupShell className="is-booting" aria-hidden role="presentation" />;
  }

  const shellClass = [
    "dai-shell",
    entered ? "is-entered" : "is-revealing",
    dropHover ? "is-drop-hover" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <ChromePopupShell
      density="compact"
      className={shellClass}
      role="dialog"
      aria-label="添加图标"
    >
      <div className="dai-toolbar">
        <button
          type="button"
          className="dai-icon-btn"
          aria-label="关闭"
          onClick={() => void closeSelf()}
        >
          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden>
            <path
              d="M4 4l8 8M12 4l-8 8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
            />
          </svg>
        </button>
        <button
          type="button"
          className="dai-icon-btn"
          aria-label="浏览添加程序"
          title="浏览添加程序"
          disabled={busy}
          onClick={() => void browseFiles()}
        >
          <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden>
            <path
              d="M5 3.5h6.2L13.5 6H15a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 15 17H5a1.5 1.5 0 0 1-1.5-1.5v-12A1.5 1.5 0 0 1 5 3.5z"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinejoin="round"
            />
            <circle cx="14.2" cy="14.2" r="3.2" fill="var(--glass-panel-bg, #2a2a2c)" stroke="currentColor" strokeWidth="1.3" />
            <path d="M14.2 12.6v3.2M12.6 14.2h3.2" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
          </svg>
        </button>
      </div>

      <div className="dai-hero">
        <div className="dai-hero-title">添加显示名称</div>
        <div className="dai-hero-hint">点击右上角按钮添加图标</div>
      </div>

      <div className="dai-section-label">系统快捷</div>
      <div className="dai-list" role="menu">
        {presets.map((p) => {
          const locked = p.present && (p.id === "startmenu" || p.id === "trash");
          return (
            <button
              key={p.id}
              type="button"
              className={`dai-row${locked ? " is-disabled" : ""}${p.present ? " is-present" : ""}`}
              role="menuitem"
              disabled={locked || busy}
              onClick={() => void pick(p)}
            >
              <span className="dai-glyph" aria-hidden>
                <PresetGlyph id={p.id} />
              </span>
              <span className="dai-label">{p.label}</span>
              {p.present ? (
                <span className="dai-badge" aria-hidden>
                  已添加
                </span>
              ) : null}
            </button>
          );
        })}
      </div>

      <label className="dai-mask">
        <input
          type="checkbox"
          checked={useIconMask}
          onChange={(e) => setUseIconMask(e.target.checked)}
        />
        <span>使用图标遮罩</span>
      </label>

      <div className="dai-drop-hint">{dropHover ? "松开以添加" : "可将文件拖放到此处"}</div>
    </ChromePopupShell>
  );
}
