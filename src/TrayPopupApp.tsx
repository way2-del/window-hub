import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  createTrayGlyphCache,
  isTrayPinned,
  isTrayResident,
  trayLabel,
  trayPinKey,
  type TrayIconInfo,
  type TrayPrefs,
} from "./components/TrayCluster";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { fitPopupToContent } from "./popupFit";
import { armTrayLeftClick, fireTrayLeftDouble, invokeTrayRightClick } from "./trayInvoke";

const POPUP_W = 280;
const glyphCache = createTrayGlyphCache();

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

async function clickTray(icon: TrayIconInfo, action: "left" | "right" | "left-double") {
  try {
    if (action === "right") {
      await invokeTrayRightClick(icon);
    } else if (action === "left-double") {
      fireTrayLeftDouble(icon);
    } else {
      armTrayLeftClick(icon);
    }
  } catch (e) {
    console.error(e);
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
  // Soft reassert once — Rust already applied material on create/reuse.
  await invoke("apply_window_effect", {}).catch(() => undefined);
}

export default function TrayPopupApp() {
  const [boot, setBoot] = useState<{
    icons: TrayIconInfo[];
    pinned: string[];
  } | null>(null);
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [entered, setEntered] = useState(false);
  const revealGen = useRef(0);
  const reuseArmedRef = useRef(false);
  const enteredRef = useRef(false);
  enteredRef.current = entered;

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    const start = async () => {
      await syncGlass();
      if (cancelled) return;
      try {
        const [list, prefs] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
        ]);
        if (cancelled) return;
        const merged = glyphCache.merge(list);
        setEntered(false);
        setIcons(merged);
        setPinned(prefs.pinned ?? []);
        setBoot({ icons: merged, pinned: prefs.pinned ?? [] });
        const need = merged
          .filter((i) => !i.icon_png_base64)
          .map((i) => i.id)
          .slice(0, 64);
        if (need.length > 0) {
          void invoke<Record<string, string>>("get_tray_icon_glyphs", { ids: need })
            .then((map) => {
              if (cancelled) return;
              if (glyphCache.ingest(map ?? {}, merged)) {
                setIcons((prev) => glyphCache.merge(prev));
              }
            })
            .catch(() => undefined);
        }
      } catch {
        if (cancelled) return;
        setEntered(false);
        setIcons([]);
        setPinned([]);
        setBoot({ icons: [], pinned: [] });
      }
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

    void listen("tray-popup-opened", () => {
      if (!reuseArmedRef.current) return;
      void start();
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setIcons(glyphCache.merge(ev.payload ?? []));
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void listen<TrayPrefs>("tray-prefs", (ev) => {
      setPinned(ev.payload.pinned ?? []);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    // No 2s list_tray_icons poll — full PNG hydrate + merge was freezing the popup.

    let unFocus: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged((ev) => {
        // Ignore blur until slide reveal finished — intermediate show/focus
        // handoff must not close the popup.
        if (!ev.payload && reuseArmedRef.current && enteredRef.current) {
          void invoke("close_tray_popup").catch(() => undefined);
        }
      })
      .then((fn) => {
        unFocus = fn;
      });

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      unsubs.forEach((fn) => fn());
      unFocus?.();
    };
  }, []);

  useLayoutEffect(() => {
    if (!boot) return;
    let cancelled = false;
    const gen = ++revealGen.current;
    void (async () => {
      try {
        // One setSize + show — never 15-frame slideReveal (SetWindowPos storm → 未响应).
        await fitPopupToContent({
          width: POPUP_W,
          selector: ".tray-popup-shell",
          minHeight: 72,
          maxHeight: 520,
        });
        if (cancelled || gen !== revealGen.current) return;
        const win = getCurrentWindow();
        await win.show();
        await win.setFocus();
        reuseArmedRef.current = true;
        setEntered(true);
      } catch (e) {
        console.error("[TrayPopup]", e);
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

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const liveTrayKeys = useMemo(
    () => icons.map((i) => trayPinKey(i)).filter(Boolean),
    [icons],
  );
  const pinnedIcons = useMemo(() => {
    const list = icons.filter(
      (i) => isTrayResident(i) || isTrayPinned(i, pinnedSet, liveTrayKeys),
    );
    return [
      ...list.filter((i) => !isTrayResident(i)),
      ...list.filter((i) => isTrayResident(i)),
    ];
  }, [icons, pinnedSet, liveTrayKeys]);
  const overflowIcons = useMemo(
    () =>
      icons.filter(
        (i) => !isTrayResident(i) && !isTrayPinned(i, pinnedSet, liveTrayKeys),
      ),
    [icons, pinnedSet, liveTrayKeys],
  );

  if (!boot) {
    return <div className="tray-popup-shell is-booting" aria-hidden />;
  }

  const shellClass = [
    "tray-popup-shell",
    "is-origin-down",
    entered ? "is-entered" : "is-revealing",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={shellClass} role="menu">
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
                    onDoubleClick={(e) => {
                      e.preventDefault();
                      void clickTray(icon, "left-double");
                    }}
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
                    onDoubleClick={(e) => {
                      e.preventDefault();
                      void clickTray(icon, "left-double");
                    }}
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
