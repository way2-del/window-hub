import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./components/StatusMenu.css";

/** Keep in sync with `STATUS_MENU_POPUP_W` in commands.rs */
const STATUS_MENU_W = 220;

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

export default function StatusMenuPopupApp() {
  const [taskbarVisible, setTaskbarVisible] = useState(true);
  const shellRef = useRef<HTMLDivElement | null>(null);
  const lastFitH = useRef(0);

  /** Shrink/grow the HWND to the menu content so the glass panel has no empty tail. */
  useLayoutEffect(() => {
    const el = shellRef.current;
    if (!el) return;

    const fit = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h <= 0 || h === lastFitH.current) return;
      lastFitH.current = h;
      void getCurrentWindow()
        .setSize(new LogicalSize(STATUS_MENU_W, h))
        .catch(() => undefined);
    };

    fit();
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);

    let unlisten: (() => void) | undefined;
    void listen("status-menu-popup-opened", () => {
      lastFitH.current = 0;
      // After Rust restores the initial size, re-measure on next frame.
      requestAnimationFrame(fit);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      ro.disconnect();
      unlisten?.();
    };
  }, [taskbarVisible]);

  useEffect(() => {
    void (async () => {
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
    })();

    void invoke<boolean>("is_system_taskbar_visible")
      .then(setTaskbarVisible)
      .catch(() => undefined);

    const retryA = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 120);
    const retryB = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 350);

    let cancelled = false;
    const unsubs: Array<() => void> = [];

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

  return (
    <div ref={shellRef} className="status-menu-shell" role="menu">
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
      <div className="status-menu-sep" role="separator" />
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "taskmgr" });
          })
        }
      >
        任务管理器
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "device-manager" });
          })
        }
      >
        设备管理器
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "control-panel" });
          })
        }
      >
        控制面板
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "windows-settings" });
          })
        }
      >
        系统设置
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "env-vars" });
          })
        }
      >
        环境变量
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "cmd-admin" });
          })
        }
      >
        管理员 CMD
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "powershell-admin" });
          })
        }
      >
        管理员 PowerShell
      </button>
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_system_tool", { kind: "terminal" });
          })
        }
      >
        终端
      </button>
      <div className="status-menu-sep" role="separator" />
      <button
        type="button"
        className="status-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("set_system_taskbar_visible", {
              visible: !taskbarVisible,
            });
          })
        }
      >
        {taskbarVisible ? "隐藏系统任务栏" : "显示系统任务栏"}
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
