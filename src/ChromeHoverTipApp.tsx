import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalPosition, LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import {
  normalizeGlassKind,
  subscribeSystemDark,
  syncGlassCss,
  type GlassPrefs,
} from "./glassPrefs";

type TipPayload = {
  lines: string[];
  x: number;
  y: number;
  placement?: "above" | "below" | string | null;
  imageJpegBase64?: string | null;
  iconPngBase64?: string | null;
  hwnd?: number | null;
  itemId?: string | null;
};

function normalizeTip(p: Partial<TipPayload> | null | undefined): TipPayload | null {
  if (!p) return null;
  const lines = Array.isArray(p.lines)
    ? p.lines.map(String).filter(Boolean).slice(0, 8)
    : [];
  const imageJpegBase64 = (p.imageJpegBase64 || "").trim() || undefined;
  const iconPngBase64 = (p.iconPngBase64 || "").trim() || undefined;
  if (!lines.length && !imageJpegBase64) return null;
  const hwndRaw = p.hwnd;
  const hwnd =
    typeof hwndRaw === "number" && Number.isFinite(hwndRaw) && hwndRaw !== 0
      ? Math.trunc(hwndRaw)
      : undefined;
  const itemId = (p.itemId || "").trim() || undefined;
  return {
    lines,
    x: Number(p.x) || 0,
    y: Number(p.y) || 0,
    placement: p.placement === "above" ? "above" : "below",
    imageJpegBase64,
    iconPngBase64,
    hwnd,
    itemId,
  };
}

function blobFp(s: string | null | undefined): string {
  if (!s) return "";
  const n = s.length;
  if (n <= 64) return `${n}:${s}`;
  return `${n}:${s.slice(0, 32)}:${s.slice(-32)}`;
}

function tipPayloadEq(a: TipPayload | null, b: TipPayload | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.lines.length !== b.lines.length) return false;
  for (let i = 0; i < a.lines.length; i++) {
    if (a.lines[i] !== b.lines[i]) return false;
  }
  return (
    blobFp(a.imageJpegBase64) === blobFp(b.imageJpegBase64) &&
    blobFp(a.iconPngBase64) === blobFp(b.iconPngBase64) &&
    a.hwnd === b.hwnd &&
    a.itemId === b.itemId &&
    a.placement === b.placement &&
    Math.abs(a.x - b.x) < 0.75 &&
    Math.abs(a.y - b.y) < 0.75
  );
}

/** Match PluginPopupHost: glass CSS tokens + DWM material on this HWND. */
async function applyTipGlass() {
  try {
    const prefs = await invoke<GlassPrefs>("get_material_prefs");
    await syncGlassCss({ ...prefs, kind: normalizeGlassKind(prefs.kind) });
  } catch {
    await syncGlassCss({ kind: "mica-alt", dark: null });
  }
  await invoke("apply_window_effect", {}).catch(() => undefined);
}

/** Snap logical size up to whole device pixels so DWM HWND edges stay flush. */
function snapCssPx(n: number): number {
  const dpr = window.devicePixelRatio || 1;
  return Math.max(1, Math.ceil(n * dpr) / dpr);
}

async function fitTipWindow(box: HTMLElement, tip: TipPayload) {
  const root = box.closest(".chrome-hover-tip-root") as HTMLElement | null;
  if (root) {
    root.style.width = "max-content";
    root.style.height = "max-content";
  }
  box.style.width = "max-content";
  box.style.height = "auto";
  // Preview width follows real window aspect (set on img); do not clamp tip width.
  box.style.maxWidth = tip.imageJpegBase64 ? "none" : "360px";
  void box.offsetWidth;

  const rect = box.getBoundingClientRect();
  const w = snapCssPx(Math.max(48, rect.width));
  const h = snapCssPx(Math.max(28, rect.height));
  const above = tip.placement === "above";
  // above: tip.y is the bottom edge of the tip window; below: tip.y is the top.
  const top = above ? Math.max(0, tip.y - h) : Math.max(0, tip.y);

  const win = getCurrentWindow();
  await win.setSize(new LogicalSize(w, h));
  await win.setPosition(new LogicalPosition(Math.max(4, tip.x - w / 2), top));
  const interactive = Boolean(tip.imageJpegBase64 && tip.hwnd);
  await win.setIgnoreCursorEvents(!interactive);
  await invoke("apply_window_effect", {}).catch(() => undefined);

  if (root) {
    root.style.width = "100%";
    root.style.height = "100%";
  }
  box.style.width = "100%";
  box.style.height = "100%";
  box.style.maxWidth = "none";
}

export default function ChromeHoverTipApp() {
  const [tip, setTip] = useState<TipPayload | null>(null);
  const [rightHover, setRightHover] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  const genRef = useRef(0);

  useEffect(() => {
    void applyTipGlass();
    const unDark = subscribeSystemDark(() => {
      void applyTipGlass();
    });
    let unShow: (() => void) | undefined;
    let unHide: (() => void) | undefined;
    let unMat: (() => void) | undefined;
    void (async () => {
      try {
        const initial = await invoke<TipPayload | null>("get_chrome_hover_tip");
        setTip(normalizeTip(initial));
      } catch {
        /* noop */
      }
      unShow = await listen<TipPayload>("chrome-hover-tip-show", (ev) => {
        const next = normalizeTip(ev.payload);
        setTip((prev) => {
          if (tipPayloadEq(prev, next)) return prev;
          return next;
        });
        setRightHover(false);
        if (next) void applyTipGlass();
      });
      unHide = await listen("chrome-hover-tip-hide", () => {
        setRightHover(false);
        setTip(null);
      });
      unMat = await listen<GlassPrefs>("material-prefs", (ev) => {
        void syncGlassCss({
          ...ev.payload,
          kind: normalizeGlassKind(ev.payload?.kind),
        });
        void invoke("apply_window_effect", {}).catch(() => undefined);
      });
    })();
    return () => {
      unDark();
      unShow?.();
      unHide?.();
      unMat?.();
    };
  }, []);

  useEffect(() => {
    if (!tip) return;
    const gen = ++genRef.current;
    const timers: number[] = [];
    const run = () => {
      if (gen !== genRef.current) return;
      const el = boxRef.current;
      if (!el) return;
      void fitTipWindow(el, tip).catch(() => undefined);
    };
    // Fit immediately so leave-watch hit-tests the real tip rect (not the 160×48 stub).
    // Delayed fit left a gap above the Dock → tip vanished before clicks landed.
    timers.push(window.setTimeout(run, 0));
    timers.push(window.setTimeout(run, 48));
    return () => {
      for (const id of timers) window.clearTimeout(id);
    };
  }, [tip]);

  if (!tip) {
    return <div className="chrome-hover-tip-root" aria-hidden />;
  }

  const interactive = Boolean(tip.imageJpegBase64 && tip.hwnd);
  const title = tip.lines[0] || "";
  const extraLines = tip.lines.slice(1);
  const iconSrc = tip.iconPngBase64
    ? `data:image/png;base64,${tip.iconPngBase64}`
    : null;

  async function onCloseWindow() {
    if (!tip?.hwnd) return;
    try {
      await invoke("close_window_hwnd", { hwnd: tip.hwnd });
    } catch (e) {
      console.error("[ChromeHoverTip] close", e);
    }
    try {
      await invoke("close_chrome_hover_tip");
    } catch {
      /* noop */
    }
  }

  async function onActivateApp() {
    if (!tip) return;
    const itemId = (tip.itemId || "").trim();
    try {
      if (itemId) {
        await invoke("dock_launch_item", { itemId });
      } else if (tip.hwnd) {
        await invoke("focus_open_window", { id: `hwnd:${tip.hwnd}` });
      } else {
        return;
      }
    } catch (e) {
      console.error("[ChromeHoverTip] activate", e);
    }
    try {
      await invoke("close_chrome_hover_tip");
    } catch {
      /* noop */
    }
  }

  return (
    <div
      className={`chrome-hover-tip-root${tip.imageJpegBase64 ? " has-preview" : ""}${interactive ? " is-interactive" : ""}`}
    >
      <div
        ref={boxRef}
        className="chrome-hover-tip-box"
        role={interactive ? "button" : "tooltip"}
        onPointerMove={
          interactive
            ? (e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setRightHover(e.clientX >= r.left + r.width * 0.62);
              }
            : undefined
        }
        onPointerLeave={interactive ? () => setRightHover(false) : undefined}
        onClick={
          interactive
            ? (e) => {
                if ((e.target as HTMLElement).closest?.(".chrome-hover-tip-close")) {
                  return;
                }
                e.preventDefault();
                void onActivateApp();
              }
            : undefined
        }
      >
        {tip.imageJpegBase64 ? (
          <>
            <div className="chrome-hover-tip-title-row">
              {iconSrc ? (
                <img
                  className="chrome-hover-tip-icon"
                  src={iconSrc}
                  alt=""
                  draggable={false}
                />
              ) : null}
              <div className="chrome-hover-tip-line is-lead chrome-hover-tip-title">{title}</div>
              {interactive ? (
                <button
                  type="button"
                  className={`chrome-hover-tip-close${rightHover ? " is-visible" : ""}`}
                  aria-label="关闭窗口"
                  tabIndex={-1}
                  onClick={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    void onCloseWindow();
                  }}
                >
                  <svg width="10" height="10" viewBox="0 0 12 12" aria-hidden>
                    <path
                      d="M2.2 2.2l7.6 7.6M9.8 2.2L2.2 9.8"
                      stroke="currentColor"
                      strokeWidth="1.6"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              ) : null}
            </div>
            <div className="chrome-hover-tip-preview-wrap">
              <img
                key={blobFp(tip.imageJpegBase64)}
                className="chrome-hover-tip-preview"
                src={`data:image/jpeg;base64,${tip.imageJpegBase64}`}
                alt=""
                draggable={false}
                onLoad={(e) => {
                  // Same height for all; width = height × real capture aspect (full window).
                  const img = e.currentTarget;
                  const nw = img.naturalWidth;
                  const nh = img.naturalHeight;
                  if (nw > 0 && nh > 0) {
                    const wrap = img.parentElement;
                    const hCss =
                      (wrap && getComputedStyle(wrap).getPropertyValue("--dock-preview-h")) ||
                      "160px";
                    let h = Math.max(1, parseFloat(hCss) || 160);
                    let w = Math.max(1, Math.round((h * nw) / nh));
                    // Only if tip would exceed most of the screen: scale both axes
                    // so the full window still fits (never crop / never distort).
                    const maxW = Math.max(
                      160,
                      Math.floor((window.screen?.availWidth || window.innerWidth || 1200) * 0.72),
                    );
                    if (w > maxW) {
                      const s = maxW / w;
                      w = maxW;
                      h = Math.max(48, Math.round(h * s));
                    }
                    img.style.width = `${w}px`;
                    img.style.height = `${h}px`;
                    if (wrap) {
                      wrap.style.width = `${w}px`;
                      wrap.style.height = `${h}px`;
                    }
                  }
                  const el = boxRef.current;
                  if (el && tip) void fitTipWindow(el, tip).catch(() => undefined);
                }}
              />
            </div>
            {extraLines.map((line, i) => (
              <div key={`${i}-${line.slice(0, 12)}`} className="chrome-hover-tip-line">
                {line}
              </div>
            ))}
          </>
        ) : (
          <>
            {iconSrc ? (
              <div className="chrome-hover-tip-title-row">
                <img
                  className="chrome-hover-tip-icon"
                  src={iconSrc}
                  alt=""
                  draggable={false}
                />
                {title ? (
                  <div className="chrome-hover-tip-line is-lead chrome-hover-tip-title">
                    {title}
                  </div>
                ) : null}
              </div>
            ) : null}
            {(iconSrc ? tip.lines.slice(1) : tip.lines).map((line, i) => (
              <div
                key={`${i}-${line.slice(0, 12)}`}
                className={`chrome-hover-tip-line${!iconSrc && i === 0 ? " is-lead" : ""}`}
              >
                {line}
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
