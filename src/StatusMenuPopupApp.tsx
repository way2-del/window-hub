import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { fitPopupToContent } from "./popupFit";
import { bootstrapPlugins } from "./plugins/bootstrap";
import "./components/StatusMenu.css";

declare global {
  interface Window {
    __WH_STATUS_MENU_FROM_DOCK__?: boolean;
    __WH_STATUS_MENU_ITEM_ID__?: string | null;
    __WH_STATUS_MENU_AFTER_ITEM_ID__?: string | null;
    __WH_STATUS_MENU_PIN_BOTTOM__?: number | null;
  }
}

const POPUP_W = 200;

type OpenPayload = {
  fromDock?: boolean;
  itemId?: string | null;
  afterItemId?: string | null;
  pinBottom?: number | null;
};

/** Captured once per open (dock menus open upward). */
let pinBottomCached: number | null | undefined;
let fromDockCached: boolean | undefined;
let dockItemIdCached: string | null | undefined;
let afterItemIdCached: string | null | undefined;

async function closeSelf() {
  try {
    await invoke("close_status_menu_popup");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

async function run(action: () => Promise<void>) {
  try {
    await action();
  } catch (e) {
    console.error(e);
  } finally {
    await closeSelf();
  }
}

function resetOpenCaches() {
  pinBottomCached = undefined;
  fromDockCached = undefined;
  dockItemIdCached = undefined;
  afterItemIdCached = undefined;
}

function applyPayload(payload?: OpenPayload | null) {
  if (!payload) return;
  if (typeof payload.fromDock === "boolean") {
    fromDockCached = payload.fromDock;
    window.__WH_STATUS_MENU_FROM_DOCK__ = payload.fromDock;
  }
  if (payload.itemId !== undefined) {
    const id = (payload.itemId ?? "").trim() || null;
    dockItemIdCached = id;
    window.__WH_STATUS_MENU_ITEM_ID__ = id;
  }
  if (payload.afterItemId !== undefined) {
    const id = (payload.afterItemId ?? "").trim() || null;
    afterItemIdCached = id;
    window.__WH_STATUS_MENU_AFTER_ITEM_ID__ = id;
  }
  if (payload.pinBottom !== undefined) {
    const n = payload.pinBottom;
    pinBottomCached = typeof n === "number" && Number.isFinite(n) ? n : null;
    window.__WH_STATUS_MENU_PIN_BOTTOM__ = pinBottomCached;
  }
}

function readPinBottom(): number | null {
  if (pinBottomCached !== undefined) return pinBottomCached;
  const fromWin = window.__WH_STATUS_MENU_PIN_BOTTOM__;
  if (typeof fromWin === "number" && Number.isFinite(fromWin)) {
    pinBottomCached = fromWin;
    return fromWin;
  }
  try {
    const raw = sessionStorage.getItem("wh.statusMenu.pinBottom");
    sessionStorage.removeItem("wh.statusMenu.pinBottom");
    if (!raw) {
      pinBottomCached = null;
      return null;
    }
    const n = Number(raw);
    pinBottomCached = Number.isFinite(n) ? n : null;
    return pinBottomCached;
  } catch {
    pinBottomCached = null;
    return null;
  }
}

function readFromDock(): boolean {
  if (fromDockCached !== undefined) return fromDockCached;
  if (typeof window.__WH_STATUS_MENU_FROM_DOCK__ === "boolean") {
    fromDockCached = window.__WH_STATUS_MENU_FROM_DOCK__;
    return fromDockCached;
  }
  try {
    const raw = sessionStorage.getItem("wh.statusMenu.fromDock");
    sessionStorage.removeItem("wh.statusMenu.fromDock");
    fromDockCached = raw === "1";
    return fromDockCached;
  } catch {
    fromDockCached = false;
    return false;
  }
}

function readDockItemId(): string | null {
  if (dockItemIdCached !== undefined) return dockItemIdCached;
  const fromWin = window.__WH_STATUS_MENU_ITEM_ID__;
  if (typeof fromWin === "string" && fromWin.trim()) {
    dockItemIdCached = fromWin.trim();
    return dockItemIdCached;
  }
  try {
    const raw = (sessionStorage.getItem("wh.statusMenu.itemId") || "").trim();
    sessionStorage.removeItem("wh.statusMenu.itemId");
    dockItemIdCached = raw || null;
    return dockItemIdCached;
  } catch {
    dockItemIdCached = null;
    return null;
  }
}

/** Gap anchor for inserting a separator (from open payload / init script). */
function readAfterItemId(): string | null {
  if (afterItemIdCached !== undefined) return afterItemIdCached;
  const fromWin = window.__WH_STATUS_MENU_AFTER_ITEM_ID__;
  if (typeof fromWin === "string" && fromWin.trim()) {
    afterItemIdCached = fromWin.trim();
    return afterItemIdCached;
  }
  if (fromWin === null) {
    afterItemIdCached = null;
    return null;
  }
  try {
    const raw = (sessionStorage.getItem("wh.statusMenu.afterItemId") || "").trim();
    sessionStorage.removeItem("wh.statusMenu.afterItemId");
    afterItemIdCached = raw || null;
    return afterItemIdCached;
  } catch {
    afterItemIdCached = null;
    return null;
  }
}

function menuOrigin(): "up" | "down" {
  return readPinBottom() != null ? "up" : "down";
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

/** Fit to content then show — no 2px slide (that left a light window-frame strip). */
async function revealFitted(_direction: "up" | "down") {
  const pinBottom = readPinBottom();
  await fitPopupToContent({
    width: POPUP_W,
    selector: ".status-menu-shell",
    // Wrap content tightly — do not floor at 72 (leaves a hollow top on 1-item menus).
    minHeight: 36,
    maxHeight: 480,
    pinBottom,
  });
  const win = getCurrentWindow();
  await win.show();
  await win.setFocus();
  await invoke("apply_window_effect", {}).catch(() => undefined);
}

type DockMenuItem = { id: string; kind: string };

export default function StatusMenuPopupApp() {
  const [boot, setBoot] = useState<{
    hiddenCount: number;
    origin: "up" | "down";
    fromDock: boolean;
    dockItemId: string | null;
    dockItemKind: string | null;
    afterItemId: string | null;
    windowCount: number;
  } | null>(null);
  const [entered, setEntered] = useState(false);
  const revealGen = useRef(0);
  /** After first reveal, `status-menu-popup-opened` means HWND reuse (not initial emit). */
  const reuseArmedRef = useRef(false);
  const openingEditorRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    const start = async () => {
      await syncGlass();
      await bootstrapPlugins();
      if (cancelled) return;
      let n = 0;
      try {
        n = await invoke<number>("get_dock_hidden_count");
      } catch {
        n = 0;
      }
      const dockItemId = readDockItemId();
      const afterItemId = readAfterItemId() ?? dockItemId;
      let dockItemKind: string | null = null;
      let windowCount = 0;
      if (dockItemId) {
        try {
          const items = await invoke<DockMenuItem[]>("get_dock_display_items");
          dockItemKind = items.find((it) => it.id === dockItemId)?.kind ?? null;
        } catch {
          dockItemKind = null;
        }
        try {
          windowCount = await invoke<number>("dock_item_window_count", {
            itemId: dockItemId,
          });
        } catch {
          windowCount = 0;
        }
      }
      if (cancelled) return;
      setEntered(false);
      openingEditorRef.current = false;
      setBoot({
        hiddenCount: n,
        origin: menuOrigin(),
        fromDock: readFromDock(),
        dockItemId,
        dockItemKind,
        afterItemId,
        windowCount,
      });
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

    void listen<OpenPayload>("status-menu-popup-opened", (ev) => {
      if (!reuseArmedRef.current) {
        applyPayload(ev.payload);
        return;
      }
      resetOpenCaches();
      applyPayload(ev.payload);
      void start();
    }).then((fn) => {
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
    if (!boot) return;
    let cancelled = false;
    const gen = ++revealGen.current;
    const direction = boot.origin;
    void (async () => {
      try {
        await revealFitted(direction);
        if (cancelled || gen !== revealGen.current) return;
        reuseArmedRef.current = true;
        setEntered(true);
      } catch (e) {
        console.error("[StatusMenuPopup]", e);
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
  }, [boot]);

  if (!boot) {
    return <div className="status-menu-shell is-booting" aria-hidden />;
  }

  const {
    hiddenCount,
    origin,
    fromDock,
    dockItemId,
    dockItemKind,
    afterItemId,
    windowCount,
  } = boot;
  const isRunningTile = !!dockItemId?.startsWith("running:");
  const isSeparator = dockItemKind === "separator";
  /** Dock 图标/分割线：仅应用相关项。空 Dock / 岛标题：系统设置菜单。 */
  const itemMenu = fromDock && !!dockItemId;
  const systemMenu = !itemMenu;
  const canCloseWindows =
    itemMenu && !isSeparator && windowCount > 0 && (isRunningTile || dockItemKind === "app");
  const shellClass = [
    "status-menu-shell",
    origin === "up" ? "is-origin-up" : "is-origin-down",
    entered ? "is-entered" : "is-revealing",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={shellClass} role="menu">
      {itemMenu ? (
        <>
          {isRunningTile ? (
            <button
              type="button"
              className="status-menu-item"
              role="menuitem"
              onClick={() =>
                void run(async () => {
                  await invoke("dock_pin_item", { itemId: dockItemId });
                })
              }
            >
              固定到 Dock
            </button>
          ) : isSeparator ? (
            <button
              type="button"
              className="status-menu-item is-danger"
              role="menuitem"
              onClick={() =>
                void run(async () => {
                  await invoke("dock_unpin_item", { itemId: dockItemId });
                })
              }
            >
              删除分割线
            </button>
          ) : (
            <>
              <button
                type="button"
                className="status-menu-item"
                role="menuitem"
                disabled={openingEditorRef.current}
                onClick={() => {
                  if (openingEditorRef.current) return;
                  openingEditorRef.current = true;
                  void (async () => {
                    try {
                      // Open editor first — closing this HWND first aborts the invoke.
                      await invoke("open_dock_icon_editor", {
                        itemId: dockItemId || null,
                      });
                    } catch (e) {
                      console.error("[StatusMenuPopup] open editor", e);
                      openingEditorRef.current = false;
                      return;
                    }
                    await closeSelf();
                  })();
                }}
              >
                修改图标
              </button>
              <button
                type="button"
                className="status-menu-item"
                role="menuitem"
                onClick={() =>
                  void run(async () => {
                    await invoke("dock_add_separator", {
                      afterItemId: afterItemId || dockItemId,
                    });
                  })
                }
              >
                在右侧添加分割线
              </button>
              {dockItemKind === "app" ? (
                <button
                  type="button"
                  className="status-menu-item"
                  role="menuitem"
                  onClick={() =>
                    void run(async () => {
                      await invoke("dock_unpin_item", {
                        itemId: dockItemId,
                      });
                    })
                  }
                >
                  从 Dock 移除
                </button>
              ) : null}
            </>
          )}
          {canCloseWindows ? (
            <button
              type="button"
              className="status-menu-item is-danger"
              role="menuitem"
              onClick={() =>
                void run(async () => {
                  await invoke("dock_close_item_windows", {
                    itemId: dockItemId,
                  });
                })
              }
            >
              关闭窗口
            </button>
          ) : null}
        </>
      ) : null}

      {systemMenu ? (
        <>
          {fromDock ? (
            <>
              <button
                type="button"
                className="status-menu-item"
                role="menuitem"
                onClick={() =>
                  void run(async () => {
                    await invoke("dock_add_separator", {
                      afterItemId: afterItemId,
                    });
                  })
                }
              >
                在此处添加分割线
              </button>
              <div className="status-menu-sep" role="separator" />
            </>
          ) : null}
          <button
            type="button"
            className="status-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("open_settings_window");
              })
            }
          >
            设置选项
          </button>
          {hiddenCount > 0 ? (
            <>
              <div className="status-menu-sep" role="separator" />
              <button
                type="button"
                className="status-menu-item"
                role="menuitem"
                onClick={() =>
                  void run(async () => {
                    await invoke("dock_restore_hidden_items");
                  })
                }
              >
                恢复隐藏图标（{hiddenCount}）
              </button>
            </>
          ) : null}
          <div className="status-menu-sep" role="separator" />
          <button
            type="button"
            className="status-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("set_system_taskbar_visible", { visible: true });
              })
            }
          >
            显示系统任务栏
          </button>
          <button
            type="button"
            className="status-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("set_system_taskbar_visible", { visible: false });
              })
            }
          >
            隐藏系统任务栏
          </button>
          <button
            type="button"
            className="status-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("show_desktop");
              })
            }
          >
            显示桌面
          </button>
          <div className="status-menu-sep" role="separator" />
          <button
            type="button"
            className="status-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("restart_app");
              })
            }
          >
            重启 window-hub
          </button>
          <button
            type="button"
            className="status-menu-item is-danger"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                await invoke("exit_app");
              })
            }
          >
            退出 window-hub
          </button>
        </>
      ) : null}
    </div>
  );
}
