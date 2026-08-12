import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  mergeTrayIcons,
  trayLabel,
  type TrayIconInfo,
  type TrayPrefs,
} from "./components/TrayCluster";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";

function TrayGlyph({ icon }: { icon: TrayIconInfo }) {
  if (icon.icon_png_base64) {
    return (
      <img
        className="tray-glyph"
        src={`data:image/png;base64,${icon.icon_png_base64}`}
        alt=""
        draggable={false}
      />
    );
  }
  const letter = trayLabel(icon).charAt(0).toUpperCase();
  return <span className="tray-glyph tray-glyph-fallback">{letter}</span>;
}

async function clickTray(icon: TrayIconInfo, action: "left" | "right") {
  try {
    await invoke("invoke_tray_icon", {
      id: icon.id,
      hwnd: icon.hwnd,
      callbackMsg: icon.callback_msg,
      uid: icon.uid,
      version: icon.version ?? 0,
      action,
    });
  } catch (e) {
    console.error(e);
  }
}

function snapIn(setPhase: (p: "enter" | "in" | "leave") => void) {
  const root = document.querySelector(".tray-popup-shell") as HTMLElement | null;
  if (root) {
    root.style.transition = "none";
    root.style.opacity = "1";
    root.classList.remove("is-enter", "is-leave");
    root.classList.add("is-in");
  }
  setPhase("in");
}

export default function TrayPopupApp() {
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  // Always opaque — hide/show HWND only (opacity:0 + mica = stuck frosted slab).
  const [phase, setPhase] = useState<"enter" | "in" | "leave">("in");

  useEffect(() => {
    const syncGlass = (prefs: GlassPrefs) => {
      void syncGlassCss({
        kind: "mica-alt",
        dark: prefs.dark ?? null,
        acrylicAlpha: prefs.acrylicAlpha,
      });
    };

    let cancelled = false;
    const unsubs: Array<() => void> = [];
    let closing = false;

    void listen<GlassPrefs>("material-prefs", (ev) => {
      syncGlass(ev.payload);
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
          } catch {
            /* noop */
          }
        })();
      }),
    );

    void (async () => {
      // CSS first, then reveal — syncGlass after show causes a second paint flash.
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
      // Don't wait for list_tray_icons (late reveal raced Focused hide → double flash).
      void invoke("reveal_tray_popup").catch(() => undefined);

      try {
        const [list, prefs] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
        ]);
        if (!cancelled) {
          setIcons(list);
          setPinned(prefs.pinned ?? []);
        }
      } catch {
        /* noop */
      }

      try {
        unsubs.push(
          await listen<TrayIconInfo[]>("tray-icons", (ev) => {
            setIcons((prev) => mergeTrayIcons(prev, ev.payload ?? []));
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<TrayPrefs>("tray-prefs", (ev) => {
            setPinned(ev.payload.pinned ?? []);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-opened", () => {
            closing = false;
            snapIn(setPhase);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-closed", () => {
            // HWND hidden — keep opaque for next show.
            setPhase("in");
          }),
        );
      } catch {
        /* noop */
      }
    })();

    // Event-driven only — avoid 5s list_tray_icons polls on the UI thread.

    // Blur close is owned by Rust (hide + suppress). Keep Escape here.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || closing) return;
      closing = true;
      setPhase("in");
      void invoke("close_tray_popup").catch(() => undefined);
    };
    document.addEventListener("keydown", onKey);

    return () => {
      cancelled = true;
      document.removeEventListener("keydown", onKey);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const pinnedIcons = useMemo(
    () => icons.filter((i) => pinnedSet.has(i.id)),
    [icons, pinnedSet],
  );
  const overflowIcons = useMemo(
    () => icons.filter((i) => !pinnedSet.has(i.id)),
    [icons, pinnedSet],
  );

  return (
    <div className={`tray-popup-shell is-${phase}`} role="menu">
      {icons.length === 0 ? (
        <div className="tray-empty">暂无系统托盘图标</div>
      ) : (
        <>
          {overflowIcons.length > 0 && (
            <div className="tray-drop-section">
              <div className="tray-drop-label">已收纳</div>
              <div className="tray-drop-grid">
                {overflowIcons.map((icon) => (
                  <button
                    key={icon.id}
                    type="button"
                    className={`tray-drop-item${icon.flashing ? " is-flashing" : ""}`}
                    title={trayLabel(icon)}
                    onClick={() => void clickTray(icon, "left")}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      void clickTray(icon, "right");
                    }}
                  >
                    <TrayGlyph icon={icon} />
                    <span className="tray-drop-text">{trayLabel(icon)}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
          {pinnedIcons.length > 0 && (
            <div className="tray-drop-section">
              <div className="tray-drop-label">常显</div>
              <div className="tray-drop-grid">
                {pinnedIcons.map((icon) => (
                  <button
                    key={icon.id}
                    type="button"
                    className={`tray-drop-item${icon.flashing ? " is-flashing" : ""}`}
                    title={trayLabel(icon)}
                    onClick={() => void clickTray(icon, "left")}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      void clickTray(icon, "right");
                    }}
                  >
                    <TrayGlyph icon={icon} />
                    <span className="tray-drop-text">{trayLabel(icon)}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
