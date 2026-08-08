import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./components/StatusMenu.css";

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
    <div className="status-menu-shell" role="menu">
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
