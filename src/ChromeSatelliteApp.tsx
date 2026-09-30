/**
 * Secondary-monitor top chrome — mirror of primary shell strip (no island).
 * Same AmbientStrip + bar_comp HostBackdrop as main; shortcuts/status only.
 */
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  getChromePrefs,
  hydrateChromePrefs,
  subscribeChromePrefs,
} from "./chromePrefs";
import {
  hasRightShortcutsWing,
  resolveChromeRailTier,
} from "./features/chrome/dualShortcuts";
import { chromeTokens, chromeCssVars } from "./features/chrome/tokens";
import { AmbientStrip } from "./features/chrome/AmbientStrip";
import { sampleStripBands } from "./features/chrome/sampleStripBands";
import ShortcutsHost from "./components/ShortcutsHost";
import TrayCluster from "./components/TrayCluster";
import ChromeStatusCluster from "./components/ChromeStatusCluster";
import type { TopBarMode } from "./displayPlacementPrefs";

const TRAY_UI_ENABLED = true;

type Ambient = {
  r: number;
  g: number;
  b: number;
  hwnd?: number;
  width?: number;
  offset_x?: number;
  span_width?: number;
  png_base64?: string;
  windowLabel?: string;
};

type Rgb = { r: number; g: number; b: number };

function readBootMode(): TopBarMode {
  const w = window as Window & { __WH_CHROME_SAT_MODE__?: string };
  const q = new URLSearchParams(window.location.search).get("mode");
  const raw = (w.__WH_CHROME_SAT_MODE__ || q || "shortcuts").trim();
  if (raw === "full" || raw === "none") return raw;
  return "shortcuts";
}

export default function ChromeSatelliteApp() {
  const myLabel = useMemo(() => {
    try {
      return getCurrentWindow().label;
    } catch {
      return "chrome-sat";
    }
  }, []);
  const [mode, setMode] = useState<TopBarMode>(readBootMode);
  const [chromePrefs, setChromePrefsState] = useState(() => getChromePrefs());
  const [trayOpen, setTrayOpen] = useState(false);
  const [chromeStripW, setChromeStripW] = useState(0);
  const [barGlassPref, setBarGlassPref] = useState(true);
  const [barGlassDark, setBarGlassDark] = useState(true);
  const [ambient, setAmbient] = useState<Ambient>({ r: 36, g: 36, b: 38 });
  const [chromeLeft, setChromeLeft] = useState(() => chromeTokens({ r: 36, g: 36, b: 38 }));
  const [chromeRight, setChromeRight] = useState(() => chromeTokens({ r: 36, g: 36, b: 38 }));
  const [centerGap, setCenterGap] = useState(() =>
    Math.max(64, Math.min(160, Math.round((window.innerWidth || 1200) * 0.06))),
  );
  const settingsAnchorRef = useRef<HTMLDivElement>(null);
  const chromeRailTier = resolveChromeRailTier(chromePrefs);
  const rightShortcuts = hasRightShortcutsWing(chromePrefs);
  const showStatus = mode === "full" && chromeRailTier !== "dual";

  const ambientFromWindow = (ambient.hwnd ?? 0) !== 0;
  /** 与主屏一致：桌面只开 Win32 玻璃，有最大化窗口才吸色 */
  const barGlassOn = !ambientFromWindow && barGlassPref;

  useEffect(() => {
    void hydrateChromePrefs().then(setChromePrefsState);
    return subscribeChromePrefs(setChromePrefsState);
  }, []);

  useEffect(() => {
    const syncGap = () => {
      const w = window.innerWidth || 1200;
      setCenterGap(Math.max(64, Math.min(160, Math.round(w * 0.06))));
    };
    syncGap();
    window.addEventListener("resize", syncGap);
    return () => window.removeEventListener("resize", syncGap);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void invoke<{ barGlass?: boolean }>("get_island_prefs")
      .then((p) => {
        if (!cancelled && typeof p?.barGlass === "boolean") setBarGlassPref(p.barGlass);
      })
      .catch(() => undefined);
    void invoke<{ dark?: boolean | null }>("get_material_prefs")
      .then((p) => {
        if (!cancelled && typeof p?.dark === "boolean") setBarGlassDark(p.dark);
      })
      .catch(() => undefined);
    let unIsland: (() => void) | undefined;
    let unMat: (() => void) | undefined;
    void listen<{ barGlass?: boolean }>("island-prefs", (ev) => {
      if (typeof ev.payload?.barGlass === "boolean") setBarGlassPref(ev.payload.barGlass);
    }).then((u) => {
      unIsland = u;
    });
    void listen<{ dark?: boolean | null }>("material-prefs", (ev) => {
      if (typeof ev.payload?.dark === "boolean") setBarGlassDark(ev.payload.dark);
    }).then((u) => {
      unMat = u;
    });
    const t = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 320);
    return () => {
      cancelled = true;
      unIsland?.();
      unMat?.();
      window.clearTimeout(t);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: { current?: () => void } = {};
    let hwndRef = { current: 0 };
    let clearGlassTimer: { current?: number } = {};
    const clearGlassAfterCover = () => {
      // 吸色先盖在玻璃上 → 等两帧 + 短延迟再卸玻璃，杜绝透明黑底。
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
          if (cancelled) return;
          if (clearGlassTimer.current) window.clearTimeout(clearGlassTimer.current);
          clearGlassTimer.current = window.setTimeout(() => {
            if (cancelled) return;
            void invoke("apply_window_effect", {}).catch(() => undefined);
          }, 90);
        });
      });
    };
    const applyStrip = (strip: Ambient, forceMaterial: boolean) => {
      const nextHwnd = strip.hwnd ?? 0;
      const prevHwnd = hwndRef.current;
      const sceneFlip = (prevHwnd === 0) !== (nextHwnd === 0);
      hwndRef.current = nextHwnd;
      setAmbient(strip);
      if (!(forceMaterial || sceneFlip)) return;
      if (nextHwnd !== 0 && (prevHwnd === 0 || forceMaterial)) {
        // Maximize: cover glass with ribbon first; FE owns glass clear.
        clearGlassAfterCover();
        return;
      }
      // Desktop (or other): attach glass immediately.
      if (clearGlassTimer.current) window.clearTimeout(clearGlassTimer.current);
      void invoke("apply_window_effect", {}).catch(() => undefined);
    };
    void (async () => {
      try {
        const first = await invoke<Ambient>("sample_ambient_for_window");
        if (!cancelled) applyStrip(first, true);
      } catch {
        /* noop */
      }
      try {
        unlisten.current = await listen<Ambient>("ambient-color", (ev) => {
          const label = (ev.payload.windowLabel || "").trim();
          if (label !== myLabel) return;
          if (cancelled) return;
          applyStrip(ev.payload, false);
        });
      } catch {
        /* noop */
      }
    })();
    // Dual-monitor safety: poll the watcher cache (same invoke as「刷新」seed).
    // Events alone are unreliable across secondary WebView2 instances.
    const pollId = window.setInterval(() => {
      if (cancelled) return;
      void invoke<Ambient>("sample_ambient_for_window")
        .then((strip) => {
          if (cancelled) return;
          const nextHwnd = strip.hwnd ?? 0;
          const prevHwnd = hwndRef.current;
          const sceneFlip = (prevHwnd === 0) !== (nextHwnd === 0);
          if (sceneFlip) {
            applyStrip(strip, true);
            return;
          }
          setAmbient((prev) => {
            if (
              (prev.hwnd ?? 0) === nextHwnd &&
              prev.r === strip.r &&
              prev.g === strip.g &&
              prev.b === strip.b &&
              prev.png_base64 === strip.png_base64
            ) {
              return prev;
            }
            hwndRef.current = nextHwnd;
            return strip;
          });
        })
        .catch(() => undefined);
    }, 450);
    return () => {
      cancelled = true;
      unlisten.current?.();
      window.clearInterval(pollId);
      if (clearGlassTimer.current) window.clearTimeout(clearGlassTimer.current);
    };
  }, [myLabel]);

  // Backup：回桌面立刻挂玻璃；最大化由 applyStrip 盖色后再清玻璃
  useEffect(() => {
    if ((ambient.hwnd ?? 0) !== 0) return;
    void invoke("apply_window_effect", {}).catch(() => undefined);
  }, [ambient.hwnd]);

  useEffect(() => {
    let cancelled = false;
    const onDesktop = (ambient.hwnd ?? 0) === 0;
    const desktopGlass = onDesktop && barGlassPref;
    const mixGlass = (rgb: Rgb): Rgb => {
      if (!desktopGlass) return rgb;
      const tint = barGlassDark
        ? { r: 28, g: 28, b: 30 }
        : { r: 245, g: 245, b: 250 };
      const t = 0.42;
      return {
        r: Math.round(rgb.r * (1 - t) + tint.r * t),
        g: Math.round(rgb.g * (1 - t) + tint.g * t),
        b: Math.round(rgb.b * (1 - t) + tint.b * t),
      };
    };
    const fallback = mixGlass({ r: ambient.r, g: ambient.g, b: ambient.b });
    void (async () => {
      let left = fallback;
      let right = fallback;
      let center = fallback;
      if (ambient.png_base64 && (ambient.width ?? 0) > 1) {
        const bands = await sampleStripBands(ambient.png_base64);
        if (bands) {
          left = mixGlass(bands.left);
          center = mixGlass(bands.center);
          right = mixGlass(bands.right);
        }
      }
      if (cancelled) return;
      if (desktopGlass) {
        const tokens = chromeTokens(center);
        setChromeLeft(tokens);
        setChromeRight(tokens);
        return;
      }
      setChromeLeft(chromeTokens(left));
      setChromeRight(chromeTokens(right));
    })();
    return () => {
      cancelled = true;
    };
  }, [
    ambient.r,
    ambient.g,
    ambient.b,
    ambient.hwnd,
    ambient.png_base64,
    ambient.width,
    barGlassPref,
    barGlassDark,
  ]);

  useEffect(() => {
    let un: (() => void) | undefined;
    void listen<{ label?: string; mode?: string }>("chrome-sat-mode", (ev) => {
      if (ev.payload?.label && ev.payload.label !== myLabel) return;
      const m = ev.payload?.mode;
      if (m === "full" || m === "shortcuts" || m === "none") setMode(m);
    }).then((u) => {
      un = u;
    });
    return () => {
      un?.();
    };
  }, [myLabel]);

  if (mode === "none") {
    return <div className="chrome-sat-root" aria-hidden />;
  }

  // 与主屏 App.tsx stripStyle 一致：色带 + 实色兜底，避免清玻璃后透空
  const stripStyle: CSSProperties =
    ambient.png_base64 && (ambient.width ?? 0) > 1
      ? {
          backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})`,
          backgroundImage: `url(data:image/png;base64,${ambient.png_base64})`,
          backgroundRepeat: "no-repeat",
          backgroundSize:
            ambient.offset_x === 0
              ? "100% 100%"
              : ambient.span_width && ambient.span_width > 0
                ? `${ambient.span_width}px 100%`
                : "100% 100%",
          backgroundPosition:
            ambient.offset_x === 0
              ? "0 0"
              : typeof ambient.offset_x === "number"
                ? `${ambient.offset_x}px 0`
                : "0 0",
        }
      : {
          backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})`,
        };

  const barStyle: CSSProperties = {
    ["--ambient-r" as string]: String(ambient.r),
    ["--ambient-g" as string]: String(ambient.g),
    ["--ambient-b" as string]: String(ambient.b),
    // 最大化吸色：根节点也铺实色，防止 AmbientStrip 未就绪时整条透明
    ...(ambientFromWindow
      ? { backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})` }
      : null),
    ...chromeCssVars("left", chromeLeft),
    ...chromeCssVars("right", chromeRight),
  };

  return (
    <div
      className={`chrome-sat-root chrome-sat-root--${mode}${barGlassOn ? " has-bar-glass is-desktop-glass" : ""}${ambientFromWindow ? " has-ambient" : ""}`}
      style={barStyle}
    >
      {/* 吸色盖在玻璃上：实色垫底 + AmbientStrip，再由 applyStrip 卸玻璃 */}
      {ambientFromWindow ? (
        <>
          <div
            className="chrome-sat-ambient-cover"
            style={{ backgroundColor: `rgb(${ambient.r}, ${ambient.g}, ${ambient.b})` }}
            aria-hidden
          />
          <AmbientStrip style={stripStyle} />
        </>
      ) : null}
      {barGlassOn ? <div className="bar-glass" aria-hidden /> : null}
      <div ref={settingsAnchorRef} className="settings-anchor" aria-hidden />
      <ShortcutsHost
        settingsRef={settingsAnchorRef}
        islandWidth={centerGap}
        side="left"
        dualMode={rightShortcuts || mode === "shortcuts"}
      />
      {rightShortcuts || mode === "shortcuts" ? (
        <ShortcutsHost
          settingsRef={settingsAnchorRef}
          islandWidth={centerGap}
          side="right"
          dualMode
          chromeStripW={showStatus && chromeRailTier === "hybrid" ? chromeStripW : 0}
        />
      ) : null}
      {showStatus && TRAY_UI_ENABLED ? (
        <TrayCluster
          open={trayOpen}
          onOpenChange={setTrayOpen}
          compactChipsOnly={chromeRailTier === "hybrid"}
          islandWidth={centerGap}
          onRailWidthChange={
            chromeRailTier === "hybrid" ? setChromeStripW : undefined
          }
        />
      ) : showStatus ? (
        <ChromeStatusCluster />
      ) : null}
    </div>
  );
}
