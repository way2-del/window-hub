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
};

function normalizeTip(p: Partial<TipPayload> | null | undefined): TipPayload | null {
  if (!p?.lines?.length) return null;
  return {
    lines: p.lines.map(String).filter(Boolean).slice(0, 8),
    x: Number(p.x) || 0,
    y: Number(p.y) || 0,
    placement: p.placement === "above" ? "above" : "below",
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
  box.style.maxWidth = "360px";
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
  await win.setIgnoreCursorEvents(true);
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
        setTip(normalizeTip(ev.payload));
        void applyTipGlass();
      });
      unHide = await listen("chrome-hover-tip-hide", () => setTip(null));
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

  return (
    <div className="chrome-hover-tip-root">
      <div ref={boxRef} className="chrome-hover-tip-box" role="tooltip">
        {tip.lines.map((line, i) => (
          <div
            key={`${i}-${line.slice(0, 12)}`}
            className={`chrome-hover-tip-line${i === 0 ? " is-lead" : ""}`}
          >
            {line}
          </div>
        ))}
      </div>
    </div>
  );
}
