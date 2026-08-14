import { useEffect, useLayoutEffect, useState } from "react";
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
  hasSepLeft?: boolean;
  hasSepRight?: boolean;
  ephemeral?: boolean;
  path?: string;
  appId?: string;
};

function canPinPath(path: string | undefined, appId?: string): boolean {
  if ((appId || "").trim()) return true;
  const p = (path || "").trim();
  if (!p) return false;
  const base = p.split(/[/\\]/).pop()?.toLowerCase() || "";
  return base !== "applicationframehost.exe";
}

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

  // Cold create: show only after glass + menu rows are painted.
  useLayoutEffect(() => {
    if (!payload) return;
    void invoke("reveal_dock_item_menu").catch(() => undefined);
  }, [payload]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    let cancelled = false;

    void (async () => {
      // CSS first — syncGlass after show causes a second paint flash.
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

      try {
        unsubs.push(
          await listen<Payload>("dock-item-menu", (e) => {
            if (!cancelled) setPayload(e.payload);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("dock-item-menu-closed", () => {
            // HWND hidden — keep last paint so the next show is not empty/white.
          }),
        );
      } catch {
        /* noop */
      }

      // Catch cold-open emit that raced ahead of listen.
      try {
        const pending = await invoke<Payload | null>("get_dock_item_menu_payload");
        if (!cancelled && pending) setPayload(pending);
      } catch {
        /* noop */
      }
    })();

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
  const ephemeral = !!payload.ephemeral;
  const pinable = ephemeral && canPinPath(payload.path, payload.appId);

  return (
    <div className="dock-item-menu-shell" role="menu">
      <div className="dock-item-menu-title">{payload.label || "应用"}</div>
      <button
        type="button"
        className="dock-item-menu-item"
        role="menuitem"
        onClick={() =>
          void run(async () => {
            if (ephemeral) {
              if (wins[0]?.id) {
                await invoke("focus_open_window", { id: wins[0].id });
              } else if (payload.path) {
                await invoke("dock_launch_path", { path: payload.path });
              }
            } else {
              await invoke("dock_launch_item", { itemId: payload.itemId });
            }
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
              if (ephemeral) {
                for (const w of wins) {
                  await invoke("dock_close_hwnd", { hwnd: w.hwnd }).catch(() => undefined);
                }
              } else {
                await invoke("dock_close_item_windows", { itemId: payload.itemId });
              }
            })
          }
        >
          关闭窗口
        </button>
      ) : null}
      <div className="dock-item-menu-sep" role="separator" />
      {ephemeral ? (
        <button
          type="button"
          className="dock-item-menu-item"
          role="menuitem"
          disabled={!pinable}
          onClick={() =>
            void run(async () => {
              if (!pinable) return;
              const appId = (payload.appId || "").trim() || null;
              // Edge/Chrome 应用必须带 appId，否则会变成普通浏览器图标。
              if (!appId && !(payload.path || "").trim()) return;
              await invoke("dock_pin_running_app", {
                path: payload.path || "",
                label: payload.label || null,
                appId,
              });
            })
          }
        >
          固定到 Dock
        </button>
      ) : (
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
      )}
      {!ephemeral ? (
        <>
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
                if (payload.hasSepLeft) {
                  await invoke("dock_remove_adjacent_separator", {
                    itemId: payload.itemId,
                    side: "left",
                  });
                } else {
                  await invoke("dock_add_separator", {
                    beforeId: payload.itemId,
                  });
                }
              })
            }
          >
            {payload.hasSepLeft ? "移除左侧分隔线" : "在左侧添加分隔线"}
          </button>
          <button
            type="button"
            className="dock-item-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                if (payload.hasSepRight) {
                  await invoke("dock_remove_adjacent_separator", {
                    itemId: payload.itemId,
                    side: "right",
                  });
                } else {
                  await invoke("dock_add_separator", {
                    afterId: payload.itemId,
                  });
                }
              })
            }
          >
            {payload.hasSepRight ? "移除右侧分隔线" : "在右侧添加分隔线"}
          </button>
        </>
      ) : null}
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
