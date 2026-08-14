import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalPosition, LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./DockItemMenu.css";

const MENU_W = 240;

type DockWindowLite = {
  id: string;
  hwnd: number;
  title: string;
};

type DockJumpItem = {
  label: string;
  path: string;
  kind: string;
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
  recent?: DockJumpItem[];
};

function canPinPath(path: string | undefined, appId?: string): boolean {
  if ((appId || "").trim()) return true;
  const p = (path || "").trim();
  if (!p) return false;
  const base = p.split(/[/\\]/).pop()?.toLowerCase() || "";
  return base !== "applicationframehost.exe";
}

function truncateMiddle(s: string, max = 42): string {
  const t = s.trim();
  if (t.length <= max) return t;
  const keep = Math.floor((max - 1) / 2);
  return `${t.slice(0, keep)}…${t.slice(-keep)}`;
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
  const shellRef = useRef<HTMLDivElement | null>(null);
  const lastFitH = useRef(0);

  /** Shrink HWND to content so the glass panel has no empty tail (Rust size is an estimate). */
  useLayoutEffect(() => {
    if (!payload) return;
    const el = shellRef.current;
    if (!el) return;

    const fit = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h <= 0 || h === lastFitH.current) return;
      lastFitH.current = h;
      void (async () => {
        try {
          const win = getCurrentWindow();
          const factor = await win.scaleFactor();
          const size = await win.innerSize();
          const pos = await win.outerPosition();
          const curH = size.height / factor;
          const curY = pos.y / factor;
          const curX = pos.x / factor;
          // Keep the bottom edge anchored (menu sits above the dock).
          const bottom = curY + curH;
          await win.setSize(new LogicalSize(MENU_W, h));
          if (Math.abs(curH - h) >= 1) {
            await win.setPosition(new LogicalPosition(curX, bottom - h));
          }
        } catch {
          /* noop */
        }
      })();
    };

    lastFitH.current = 0;
    fit();
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);
    return () => ro.disconnect();
  }, [payload]);

  useLayoutEffect(() => {
    if (!payload) return;
    void invoke("reveal_dock_item_menu").catch(() => undefined);
  }, [payload]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    let cancelled = false;

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
        unsubs.push(await listen("dock-item-menu-closed", () => {}));
      } catch {
        /* noop */
      }

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
  const recent = (payload.recent ?? []).slice(0, 8);
  const ephemeral = !!payload.ephemeral;
  const pinable = ephemeral && canPinPath(payload.path, payload.appId);
  const isApp = payload.kind === "app" || ephemeral;
  const canNewWindow = isApp && payload.kind !== "startmenu" && payload.kind !== "trash";

  return (
    <div ref={shellRef} className="dock-item-menu-shell" role="menu">
      <div className="dock-item-menu-header">
        <div className="dock-item-menu-title">{payload.label || "应用"}</div>
      </div>

      {recent.length > 0 ? (
        <section className="dock-item-menu-section">
          <div className="dock-item-menu-section-label">最近</div>
          <div className="dock-item-menu-scroll">
            {recent.map((r) => (
              <button
                key={`${r.kind}:${r.path}`}
                type="button"
                className="dock-item-menu-item is-recent"
                role="menuitem"
                title={r.path}
                onClick={() =>
                  void run(async () => {
                    await invoke("dock_open_jump_item", {
                      itemId: payload.itemId,
                      path: r.path,
                      appPath: payload.path || null,
                    });
                  })
                }
              >
                <span className="dock-item-menu-item-label">{r.label || "未命名"}</span>
                <span className="dock-item-menu-item-sub">{truncateMiddle(r.path, 36)}</span>
              </button>
            ))}
          </div>
        </section>
      ) : null}

      {wins.length > 0 ? (
        <section className="dock-item-menu-section">
          <div className="dock-item-menu-section-label">打开的窗口</div>
          <div className="dock-item-menu-scroll">
            {wins.slice(0, 6).map((w) => (
              <button
                key={w.id}
                type="button"
                className="dock-item-menu-item"
                role="menuitem"
                title={w.title || "无标题"}
                onClick={() =>
                  void run(async () => {
                    await invoke("focus_open_window", { id: w.id });
                  })
                }
              >
                <span className="dock-item-menu-item-label">{w.title || "无标题"}</span>
              </button>
            ))}
          </div>
        </section>
      ) : null}

      <section className="dock-item-menu-section is-actions">
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
        {canNewWindow ? (
          <button
            type="button"
            className="dock-item-menu-item"
            role="menuitem"
            onClick={() =>
              void run(async () => {
                if (ephemeral) {
                  if (payload.path) {
                    await invoke("dock_launch_path", {
                      path: payload.path,
                      forceNew: true,
                    });
                  }
                } else {
                  await invoke("dock_launch_item", {
                    itemId: payload.itemId,
                    forceNew: true,
                  });
                }
              })
            }
          >
            新开窗口
          </button>
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
      </section>

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
      ) : payload.kind === "app" ? (
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
      ) : null}
    </div>
  );
}
