import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./DockItemMenu.css";

type DockWindowLite = {
  id: string;
  hwnd: number;
  title: string;
};

type Payload = {
  itemId: string;
  label: string;
  kind: string;
  windows: DockWindowLite[];
};

async function closeSelf() {
  try {
    await invoke("close_dock_item_menu");
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

export default function DockItemMenuApp() {
  const [payload, setPayload] = useState<Payload | null>(null);

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

    const unsubs: Array<() => void> = [];
    let cancelled = false;

    void listen<Payload>("dock-item-menu", (e) => {
      if (!cancelled) setPayload(e.payload);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void listen("dock-item-menu-closed", () => {
      if (!cancelled) setPayload(null);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeSystemDark(() => {
        void syncGlassCss({ kind: "mica-alt", dark: null });
      }),
    );

    const onBlur = () => {
      void closeSelf();
    };
    window.addEventListener("blur", onBlur);

    return () => {
      cancelled = true;
      window.removeEventListener("blur", onBlur);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  if (!payload) {
    return <div className="dock-item-menu-shell dock-item-menu-loading" />;
  }

  const wins = payload.windows ?? [];

  return (
    <div className="dock-item-menu-shell" role="menu">
      <div className="dock-item-menu-title">{payload.label || "应用"}</div>
      <button
        type="button"
        className="dock-item-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("dock_launch_item", { itemId: payload.itemId });
          })
        }
      >
        {wins.length > 0 ? "显示窗口" : "打开"}
      </button>
      {wins.length > 0 ? (
        <div className="dock-item-menu-sub">
          {wins.slice(0, 6).map((w) => (
            <button
              key={w.id}
              type="button"
              className="dock-item-menu-item is-sub"
              role="menuitem"
              onClick={() =>
                void run(async () => {
                  await invoke("focus_open_window", { id: w.id });
                })
              }
            >
              {w.title || "无标题"}
            </button>
          ))}
        </div>
      ) : null}
      {wins.length > 0 ? (
        <button
          type="button"
          className="dock-item-menu-item"
          role="menuitem"
          onClick={() =>
            void run(async () => {
              await invoke("dock_close_item_windows", { itemId: payload.itemId });
            })
          }
        >
          关闭窗口
        </button>
      ) : null}
      <div className="dock-item-menu-sep" role="separator" />
      <button
        type="button"
        className="dock-item-menu-item is-danger"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("dock_remove_item", { itemId: payload.itemId });
          })
        }
      >
        从 Dock 移除
      </button>
      <div className="dock-item-menu-sep" role="separator" />
      <button
        type="button"
        className="dock-item-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            const path = await invoke<string | null>("pick_dock_app_file");
            if (!path) return;
            await invoke("dock_add_app", { path, afterId: payload.itemId });
          })
        }
      >
        添加应用…
      </button>
      <button
        type="button"
        className="dock-item-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("dock_add_separator", { afterId: payload.itemId });
          })
        }
      >
        在右侧添加分隔线
      </button>
      <div className="dock-item-menu-sep" role="separator" />
      <button
        type="button"
        className="dock-item-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            await invoke("open_settings_window");
          })
        }
      >
        Dock 设置…
      </button>
    </div>
  );
}
