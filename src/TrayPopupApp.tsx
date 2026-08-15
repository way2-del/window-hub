import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import {
  SYSTEM_CHIP_LABELS,
  isTrayPinned,
  mergeTrayIcons,
  normalizeSystemChipOrder,
  normalizeSystemChips,
  normalizeSystemChipsOverflow,
  systemChipFlyoutKind,
  systemChipPlacement,
  trayLabel,
  type SystemChipKey,
  type SystemChipVisibility,
  type TrayIconInfo,
  type TrayPrefs,
} from "./components/TrayCluster";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";

/** Keep in sync with `TRAY_POPUP_W` / `TRAY_POPUP_H` in commands.rs */
const TRAY_POPUP_W = 280;
const TRAY_POPUP_MAX_H = 520;
const SYSTEM_FLYOUT_W = 280;
const TRAY_POPUP_GAP = 6;

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

async function openSystemChipFlyout(key: SystemChipKey, el: HTMLElement) {
  void invoke("suppress_system_flyout_blur", { ms: 400 });
  void invoke("close_tray_popup").catch(() => undefined);
  try {
    const win = getCurrentWindow();
    const factor = await win.scaleFactor();
    const outer = await win.outerPosition();
    const rect = el.getBoundingClientRect();
    const x = outer.x / factor + rect.right - SYSTEM_FLYOUT_W;
    const y = outer.y / factor + rect.bottom + TRAY_POPUP_GAP;
    await invoke("open_system_flyout", {
      kind: systemChipFlyoutKind(key),
      x,
      y,
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
  const [pinnedProcesses, setPinnedProcesses] = useState<string[]>([]);
  const [systemChips, setSystemChips] = useState<SystemChipVisibility>(() =>
    normalizeSystemChips(null),
  );
  const [systemChipOrder, setSystemChipOrder] = useState<SystemChipKey[]>([]);
  const [systemChipsOverflow, setSystemChipsOverflow] = useState<SystemChipKey[]>([]);
  /** Avoid fitting to the empty boot frame before the first list_tray_icons returns. */
  const [listReady, setListReady] = useState(false);
  // Always opaque — hide/show HWND only (opacity:0 + mica = stuck frosted slab).
  const [phase, setPhase] = useState<"enter" | "in" | "leave">("in");
  const shellRef = useRef<HTMLDivElement | null>(null);
  const lastFitH = useRef(0);

  /** Shrink/grow the HWND to the menu content so the glass panel has no empty tail. */
  useLayoutEffect(() => {
    if (!listReady) return;
    const el = shellRef.current;
    if (!el) return;

    const fit = () => {
      const natural = Math.ceil(Math.max(el.scrollHeight, el.getBoundingClientRect().height));
      const h = Math.min(TRAY_POPUP_MAX_H, Math.max(48, natural));
      if (h === lastFitH.current) return;
      lastFitH.current = h;
      void getCurrentWindow()
        .setSize(new LogicalSize(TRAY_POPUP_W, h))
        .catch(() => undefined);
    };

    fit();
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);

    let unlisten: (() => void) | undefined;
    void listen("tray-popup-opened", () => {
      lastFitH.current = 0;
      requestAnimationFrame(fit);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      ro.disconnect();
      unlisten?.();
    };
  }, [listReady]);

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
          setPinnedProcesses(prefs.pinned_processes ?? []);
          setSystemChips(normalizeSystemChips(prefs.system_chips));
          setSystemChipOrder(normalizeSystemChipOrder(prefs.system_chip_order));
          setSystemChipsOverflow(normalizeSystemChipsOverflow(prefs.system_chips_overflow));
          setListReady(true);
        }
      } catch {
        if (!cancelled) setListReady(true);
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
            setPinnedProcesses(ev.payload.pinned_processes ?? []);
            setSystemChips(normalizeSystemChips(ev.payload.system_chips));
            setSystemChipOrder(normalizeSystemChipOrder(ev.payload.system_chip_order));
            setSystemChipsOverflow(
              normalizeSystemChipsOverflow(ev.payload.system_chips_overflow),
            );
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

  const pinPrefs = useMemo(
    () => ({ pinned, pinned_processes: pinnedProcesses }),
    [pinned, pinnedProcesses],
  );
  const pinnedIcons = useMemo(
    () => icons.filter((i) => isTrayPinned(i, pinPrefs)),
    [icons, pinPrefs],
  );
  const overflowIcons = useMemo(
    () => icons.filter((i) => !isTrayPinned(i, pinPrefs)),
    [icons, pinPrefs],
  );
  const overflowSystemChips = useMemo(
    () =>
      systemChipOrder.filter(
        (key) => systemChipPlacement(key, systemChips, systemChipsOverflow) === "overflow",
      ),
    [systemChipOrder, systemChips, systemChipsOverflow],
  );

  const empty =
    icons.length === 0 && overflowSystemChips.length === 0;

  return (
    <div ref={shellRef} className={`tray-popup-shell is-${phase}`} role="menu">
      {empty ? (
        <div className="tray-empty">暂无系统托盘图标</div>
      ) : (
        <>
          {overflowSystemChips.length > 0 && (
            <div className="tray-drop-section">
              <div className="tray-drop-label">系统</div>
              <div className="tray-drop-grid">
                {overflowSystemChips.map((key) => (
                  <button
                    key={key}
                    type="button"
                    className="tray-drop-item"
                    title={SYSTEM_CHIP_LABELS[key]}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      void invoke("suppress_system_flyout_blur", { ms: 400 });
                    }}
                    onClick={(e) => {
                      void openSystemChipFlyout(key, e.currentTarget);
                    }}
                  >
                    <span className="tray-glyph tray-glyph-fallback">
                      {SYSTEM_CHIP_LABELS[key].charAt(0)}
                    </span>
                    <span className="tray-drop-text">{SYSTEM_CHIP_LABELS[key]}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
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
