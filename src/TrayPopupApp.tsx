import { useProgressiveGlyphs } from "./features/tray/useProgressiveGlyphs";
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
import { fitPopupToContent, schedulePopupFit } from "./popupFit";
import { armTrayLeftClick, fireTrayLeftDouble, invokeTrayRightClick } from "./trayInvoke";
import ChromePopupShell, {
  CHROME_POPUP_SHELL_SELECTOR,
} from "./features/chromePopup/ChromePopupShell";
import {
  getTrayRailFold,
  subscribeTrayRailFold,
} from "./features/chrome/trayRailFoldBus";

const POPUP_W = 280;
const TRAY_FIT = {
  width: POPUP_W,
  selector: CHROME_POPUP_SHELL_SELECTOR,
  minHeight: 72,
} as const;
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

function shellOverflows(): boolean {
  const el = document.querySelector(CHROME_POPUP_SHELL_SELECTOR);
  if (!el) return false;
  return el.scrollHeight > el.clientHeight + 1 || el.classList.contains("is-scrollable");
}

export default function TrayPopupApp() {
  const [boot, setBoot] = useState<{
    icons: TrayIconInfo[];
    pinned: string[];
  } | null>(null);
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  useProgressiveGlyphs(
    icons.filter((i) => !i.icon_png_base64).map((i) => i.id),
    (map) => {
      if (glyphCache.ingest(map, icons)) setIcons((prev) => glyphCache.merge(prev));
    },
  );
  const [entered, setEntered] = useState(false);
  /** React-owned — DOM `classList.toggle("is-scrollable")` is wiped on re-render. */
  const [scrollable, setScrollable] = useState(false);
  /** Island-squeezed rail icons (temporary; restore when island shrinks). */
  const [railFoldIds, setRailFoldIds] = useState<string[]>(
    () => getTrayRailFold().overflowIds,
  );
  const revealGen = useRef(0);
  const reuseArmedRef = useRef(false);

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const liveTrayKeys = useMemo(
    () => icons.map((i) => trayPinKey(i)).filter(Boolean),
    [icons],
  );
  const railFoldSet = useMemo(() => new Set(railFoldIds), [railFoldIds]);
  const pinnedIcons = useMemo(() => {
    const list = icons.filter(
      (i) =>
        !railFoldSet.has(i.id) &&
        (isTrayResident(i) || isTrayPinned(i, pinnedSet, liveTrayKeys)),
    );
    return [
      ...list.filter((i) => !isTrayResident(i)),
      ...list.filter((i) => isTrayResident(i)),
    ];
  }, [icons, pinnedSet, liveTrayKeys, railFoldSet]);
  const overflowIcons = useMemo(() => {
    const unpinned = icons.filter(
      (i) => !isTrayResident(i) && !isTrayPinned(i, pinnedSet, liveTrayKeys),
    );
    const islandStashed = icons.filter((i) => railFoldSet.has(i.id));
    const seen = new Set<string>();
    const out: TrayIconInfo[] = [];
    for (const icon of [...islandStashed, ...unpinned]) {
      if (seen.has(icon.id)) continue;
      seen.add(icon.id);
      out.push(icon);
    }
    return out;
  }, [icons, pinnedSet, liveTrayKeys, railFoldSet]);

  const refit = () => {
    schedulePopupFit(TRAY_FIT, [0, 40, 120]);
    window.setTimeout(() => setScrollable(shellOverflows()), 50);
    window.setTimeout(() => setScrollable(shellOverflows()), 160);
  };

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
        setScrollable(false);
        setIcons(merged);
        setPinned(prefs.pinned ?? []);
        setBoot({ icons: merged, pinned: prefs.pinned ?? [] });
      } catch {
        if (cancelled) return;
        setEntered(false);
        setScrollable(false);
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

    void listen<{ overflowIds?: string[] }>("tray-rail-fold", (ev) => {
      setRailFoldIds(
        Array.isArray(ev.payload?.overflowIds) ? ev.payload.overflowIds : [],
      );
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeTrayRailFold((p) => {
        setRailFoldIds(p.overflowIds);
      }),
    );

    setRailFoldIds(getTrayRailFold().overflowIds);

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  useLayoutEffect(() => {
    if (!boot) return;
    let cancelled = false;
    const gen = ++revealGen.current;
    void (async () => {
      try {
        const fitted = await fitPopupToContent(TRAY_FIT);
        if (cancelled || gen !== revealGen.current) return;
        setScrollable(fitted.scrollable || shellOverflows());
        const win = getCurrentWindow();
        await win.show();
        await win.setFocus();
        reuseArmedRef.current = true;
        setEntered(true);
        // Re-fit after enter — list may still be settling; keeps React `is-scrollable`.
        if (!cancelled && gen === revealGen.current) refit();
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

  // List grew while open → resize / enable scroll (otherwise bottom stays clipped).
  useLayoutEffect(() => {
    if (!boot || !entered) return;
    refit();
  }, [boot, entered, icons.length, pinnedIcons.length, overflowIcons.length]);

  if (!boot) {
    return <ChromePopupShell className="is-booting" aria-hidden role="presentation" />;
  }

  const shellClass = [
    "is-origin-down",
    entered ? "is-entered" : "is-revealing",
    scrollable ? "is-scrollable" : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <ChromePopupShell className={shellClass} role="menu" aria-label="托盘">
      {icons.length === 0 ? (
        <div className="tray-empty">暂无系统托盘图标</div>
      ) : (
        <>
          {overflowIcons.length > 0 && (
            <div className="tray-drop-section">
              <div className="tray-drop-label">已收纳</div>
              <div className="tray-drop-grid">
                {overflowIcons.map((icon, i) => (
                  <button
                    key={icon.id}
                    type="button"
                    className={`tray-drop-item is-island-stashed${icon.flashing ? " is-flashing" : ""}`}
                    style={{ animationDelay: `${Math.min(i, 8) * 28}ms` }}
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
    </ChromePopupShell>
  );
}
