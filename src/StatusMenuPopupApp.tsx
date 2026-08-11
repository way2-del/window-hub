import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { slideRevealPopup } from "./popupFit";
import "./components/StatusMenu.css";

const POPUP_W = 200;
/** Captured once per open (dock menus open upward). */
let pinBottomCached: number | null | undefined;

async function closeSelf() {
  try {
    await invoke("close_status_menu_popup");
  } catch {
    try {
      await getCurrentWindow().close();
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

function resetPinBottomCache() {
  pinBottomCached = undefined;
}

function readPinBottom(): number | null {
  if (pinBottomCached !== undefined) return pinBottomCached;
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

/** Measure while hidden → show at 2px → ease-out expand (~180ms, dock cadence). */
async function revealFitted(direction: "up" | "down") {
  const pinBottom = readPinBottom();
  await slideRevealPopup({
    width: POPUP_W,
    selector: ".status-menu-shell",
    minHeight: 72,
    maxHeight: 480,
    pinBottom,
    direction,
  });
}

export default function StatusMenuPopupApp() {
  const [boot, setBoot] = useState<{ hiddenCount: number; origin: "up" | "down" } | null>(null);
  const [entered, setEntered] = useState(false);
  const revealGen = useRef(0);
  /** After first reveal, `status-menu-popup-opened` means HWND reuse (not initial emit). */
  const reuseArmedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    const start = async () => {
      await syncGlass();
      if (cancelled) return;
      let n = 0;
      try {
        n = await invoke<number>("get_dock_hidden_count");
      } catch {
        n = 0;
      }
      if (cancelled) return;
      setEntered(false);
      setBoot({ hiddenCount: n, origin: menuOrigin() });
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

    void listen("status-menu-popup-opened", () => {
      if (!reuseArmedRef.current) return;
      resetPinBottomCache();
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

  const { hiddenCount, origin } = boot;
  const shellClass = [
    "status-menu-shell",
    origin === "up" ? "is-origin-up" : "is-origin-down",
    entered ? "is-entered" : "is-revealing",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={shellClass} role="menu">
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
    </div>
  );
}
