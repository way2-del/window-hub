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
  hwnd?: number | null;
  itemId?: string | null;
};

function normalizeTip(p: Partial<TipPayload> | null | undefined): TipPayload | null {
  if (!p) return null;
  const lines = Array.isArray(p.lines)
    ? p.lines.map(String).filter(Boolean).slice(0, 8)
    : [];
  const imageJpegBase64 = (p.imageJpegBase64 || "").trim() || undefined;
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
    hwnd,
    itemId,
  };
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
  box.style.maxWidth = tip.imageJpegBase64 ? "300px" : "360px";
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
        setRightHover(false);
        setTip(normalizeTip(ev.payload));
        void applyTipGlass();
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
    timers.push(window.setTimeout(run, 0));
    timers.push(window.setTimeout(run, 40));
    timers.push(window.setTimeout(run, 120));
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
                className="chrome-hover-tip-preview"
                src={`data:image/jpeg;base64,${tip.imageJpegBase64}`}
                alt=""
                draggable={false}
                onLoad={() => {
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
          tip.lines.map((line, i) => (
            <div
              key={`${i}-${line.slice(0, 12)}`}
              className={`chrome-hover-tip-line${i === 0 ? " is-lead" : ""}`}
            >
              {line}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
