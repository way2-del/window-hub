import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  normalizeGlassKind,
  subscribeSystemDark,
  syncGlassCss,
  type GlassPrefs,
} from "./glassPrefs";

type TipPreview = {
  jpegBase64: string;
  title: string;
  hwnd: number;
};

type TipPayload = {
  lines: string[];
  x: number;
  y: number;
  placement?: "above" | "below" | string | null;
  imageJpegBase64?: string | null;
  iconPngBase64?: string | null;
  hwnd?: number | null;
  itemId?: string | null;
  previews?: TipPreview[];
  previewHeightPx?: number | null;
  epoch?: number;
};

function normalizePreview(raw: Partial<TipPreview> | null | undefined): TipPreview | null {
  if (!raw) return null;
  const jpegBase64 = (raw.jpegBase64 || "").trim();
  const hwndRaw = raw.hwnd;
  const hwnd =
    typeof hwndRaw === "number" && Number.isFinite(hwndRaw) && hwndRaw !== 0
      ? Math.trunc(hwndRaw)
      : 0;
  if (!jpegBase64 || !hwnd) return null;
  return {
    jpegBase64,
    title: String(raw.title || "").trim(),
    hwnd,
  };
}

function normalizeTip(p: Partial<TipPayload> | null | undefined): TipPayload | null {
  if (!p) return null;
  const lines = Array.isArray(p.lines)
    ? p.lines.map(String).filter(Boolean).slice(0, 8)
    : [];
  const imageJpegBase64 = (p.imageJpegBase64 || "").trim() || undefined;
  const iconPngBase64 = (p.iconPngBase64 || "").trim() || undefined;
  let previews = Array.isArray(p.previews)
    ? p.previews.map(normalizePreview).filter((x): x is TipPreview => !!x).slice(0, 8)
    : [];
  if (!previews.length && imageJpegBase64) {
    const hwndRaw = p.hwnd;
    const hwnd =
      typeof hwndRaw === "number" && Number.isFinite(hwndRaw) && hwndRaw !== 0
        ? Math.trunc(hwndRaw)
        : 0;
    if (hwnd) {
      previews = [
        {
          jpegBase64: imageJpegBase64,
          title: lines[0] || "",
          hwnd,
        },
      ];
    }
  }
  if (!lines.length && !previews.length && !imageJpegBase64) return null;
  const hwnd = previews[0]?.hwnd;
  const itemId = (p.itemId || "").trim() || undefined;
  const epochRaw = Number(p.epoch);
  const epoch =
    Number.isFinite(epochRaw) && epochRaw > 0 ? Math.trunc(epochRaw) : 0;
  const heightRaw = Number(p.previewHeightPx);
  const previewHeightPx =
    Number.isFinite(heightRaw) && heightRaw > 0
      ? Math.min(320, Math.max(96, Math.round(heightRaw)))
      : undefined;
  return {
    lines,
    x: Number(p.x) || 0,
    y: Number(p.y) || 0,
    placement: p.placement === "above" ? "above" : "below",
    imageJpegBase64: previews[0]?.jpegBase64 || imageJpegBase64,
    iconPngBase64,
    hwnd,
    itemId,
    previews,
    previewHeightPx,
    epoch,
  };
}

function blobFp(s: string | null | undefined): string {
  if (!s) return "";
  const n = s.length;
  if (n <= 64) return `${n}:${s}`;
  return `${n}:${s.slice(0, 32)}:${s.slice(-32)}`;
}

function previewsFp(list: TipPreview[] | undefined): string {
  if (!list?.length) return "";
  return list.map((p) => `${p.hwnd}:${blobFp(p.jpegBase64)}:${p.title}`).join(";");
}

function tipPayloadEq(a: TipPayload | null, b: TipPayload | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.lines.length !== b.lines.length) return false;
  for (let i = 0; i < a.lines.length; i++) {
    if (a.lines[i] !== b.lines[i]) return false;
  }
  return (
    previewsFp(a.previews) === previewsFp(b.previews) &&
    blobFp(a.imageJpegBase64) === blobFp(b.imageJpegBase64) &&
    blobFp(a.iconPngBase64) === blobFp(b.iconPngBase64) &&
    a.hwnd === b.hwnd &&
    a.itemId === b.itemId &&
    a.previewHeightPx === b.previewHeightPx &&
    a.placement === b.placement &&
    Math.abs(a.x - b.x) < 0.75 &&
    Math.abs(a.y - b.y) < 0.75
  );
}

/** Match PluginPopupHost glass CSS tokens only — do NOT invoke apply_window_effect.
 * Backend commit applies material once while HWND is still hidden. FE deferred
 * apply_window_effect races with show and resizes the tip (bottom-right drag). */
async function applyTipGlass() {
  try {
    const prefs = await invoke<GlassPrefs>("get_material_prefs");
    await syncGlassCss({ ...prefs, kind: normalizeGlassKind(prefs.kind) });
  } catch {
    await syncGlassCss({ kind: "mica-alt", dark: null });
  }
}

/** Snap logical size up to whole device pixels so DWM HWND edges stay flush. */
function snapCssPx(n: number): number {
  const dpr = window.devicePixelRatio || 1;
  return Math.max(1, Math.ceil(n * dpr) / dpr);
}

const DEFAULT_PREVIEW_H = 160;
const TIP_PAD_X = 8;
const TIP_PAD_Y = 8;
const TIP_TITLE_H = 22;
const TIP_GAP = 6;
const CARD_GAP = 8;

/** Monitor width — never tip HWND `innerWidth` (that clamps thumbs after height grows). */
function screenLayoutWidth(): number {
  const s = window.screen;
  const w = Math.max(s?.availWidth || 0, s?.width || 0);
  return w > 200 ? w : 1200;
}

function previewHeightOf(tip: TipPayload | null | undefined): number {
  const h = tip?.previewHeightPx;
  if (typeof h === "number" && Number.isFinite(h) && h > 0) {
    return Math.min(320, Math.max(96, Math.round(h)));
  }
  return DEFAULT_PREVIEW_H;
}

function tipPreviews(tip: TipPayload): TipPreview[] {
  if (tip.previews?.length) return tip.previews;
  if (tip.imageJpegBase64 && tip.hwnd) {
    return [
      {
        jpegBase64: tip.imageJpegBase64,
        title: tip.lines[0] || "",
        hwnd: tip.hwnd,
      },
    ];
  }
  return [];
}

/**
 * Width follows JPEG aspect × preview height. Cap is per-thumb share of ~92% screen
 * so raising height grows width instead of squeezing into the tip's current HWND.
 */
function sizeOnePreview(
  img: HTMLImageElement | null,
  previewH: number,
  thumbCount: number,
): { pw: number; ph: number } {
  const nw = img?.naturalWidth || 0;
  const nh = img?.naturalHeight || 0;
  let ph = previewH;
  let pw =
    nw > 0 && nh > 0 ? Math.max(1, Math.round((ph * nw) / nh)) : Math.round(ph * (16 / 10));
  const n = Math.max(1, thumbCount);
  const screenW = screenLayoutWidth();
  const totalBudget = Math.floor(screenW * 0.92);
  const maxW = Math.max(
    160,
    Math.floor((totalBudget - CARD_GAP * (n - 1) - TIP_PAD_X * 2) / n),
  );
  if (pw > maxW) {
    const s = maxW / pw;
    pw = maxW;
    ph = Math.max(48, Math.round(ph * s));
  }
  return { pw, ph };
}

function previewTipOuterSize(
  imgs: Array<HTMLImageElement | null>,
  previewH: number,
): { w: number; h: number; sizes: Array<{ pw: number; ph: number }> } {
  const n = Math.max(1, imgs.length);
  const sizes = imgs.map((img) => sizeOnePreview(img, previewH, n));
  while (sizes.length < n) {
    sizes.push(sizeOnePreview(null, previewH, n));
  }
  const thumbsW = sizes.reduce((sum, s) => sum + s.pw, 0) + CARD_GAP * Math.max(0, n - 1);
  const ph = Math.max(...sizes.map((s) => s.ph), 1);
  const w = snapCssPx(TIP_PAD_X * 2 + thumbsW);
  const h = snapCssPx(TIP_PAD_Y * 2 + TIP_TITLE_H + TIP_GAP + ph);
  return { w, h, sizes };
}

async function fitTipWindow(box: HTMLElement, tip: TipPayload) {
  const root = box.closest(".chrome-hover-tip-root") as HTMLElement | null;
  const above = tip.placement === "above";
  const previews = tipPreviews(tip);
  const previewH = previewHeightOf(tip);

  let w: number;
  let h: number;

  if (previews.length) {
    const imgs = Array.from(
      box.querySelectorAll(".chrome-hover-tip-preview"),
    ) as HTMLImageElement[];
    const cards = Array.from(
      box.querySelectorAll(".chrome-hover-tip-preview-card"),
    ) as HTMLElement[];
    const sized = previewTipOuterSize(imgs, previewH);
    imgs.forEach((img, i) => {
      const s = sized.sizes[i];
      if (!s) return;
      img.style.width = `${s.pw}px`;
      img.style.height = `${s.ph}px`;
      const wrap = img.parentElement;
      if (wrap) {
        wrap.style.width = `${s.pw}px`;
        wrap.style.height = `${s.ph}px`;
      }
      const card = cards[i];
      if (card) {
        // Lock card to thumb width so long titles cannot widen left/right shell padding.
        card.style.width = `${s.pw}px`;
        card.style.maxWidth = `${s.pw}px`;
      }
    });
    // Measure real box (padding + cards) — formula alone drifts vs font/close metrics.
    if (root) {
      root.style.width = "max-content";
      root.style.height = "max-content";
    }
    box.style.width = "max-content";
    box.style.height = "auto";
    box.style.maxWidth = "none";
    void box.offsetWidth;
    const rect = box.getBoundingClientRect();
    w = snapCssPx(Math.max(48, Math.ceil(rect.width)));
    h = snapCssPx(Math.max(28, Math.ceil(rect.height)));
  } else {
    if (root) {
      root.style.width = "max-content";
      root.style.height = "max-content";
    }
    box.style.width = "max-content";
    box.style.height = "auto";
    box.style.maxWidth = "360px";
    void box.offsetWidth;
    const rect = box.getBoundingClientRect();
    w = snapCssPx(Math.min(360, Math.max(48, rect.width)));
    h = snapCssPx(Math.min(200, Math.max(28, rect.height)));
  }

  const top = above ? Math.max(0, tip.y - h) : Math.max(0, tip.y);
  const left = Math.max(4, tip.x - w / 2);

  await invoke("commit_chrome_hover_tip", {
    x: left,
    y: top,
    width: w,
    height: h,
    epoch: tip.epoch ?? 0,
  });

  if (root) {
    root.style.width = "100%";
    root.style.height = "100%";
  }
  // Fill HWND exactly — equal pad comes from box padding, not leftover slack.
  box.style.width = "100%";
  box.style.height = "100%";
  box.style.maxWidth = "none";
  box.style.boxSizing = "border-box";
}

export default function ChromeHoverTipApp() {
  const [tip, setTip] = useState<TipPayload | null>(null);
  /** False until HWND is fitted — hide content so stub→final never flashes. */
  const [placed, setPlaced] = useState(false);
  const [hoverHwnd, setHoverHwnd] = useState<number | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const genRef = useRef(0);
  const committedGenRef = useRef(0);
  const tipRef = useRef<TipPayload | null>(null);
  tipRef.current = tip;

  useEffect(() => {
    document.body.classList.add("is-chrome-hover-tip");
    void applyTipGlass();
    const unsubDark = subscribeSystemDark(() => {
      void applyTipGlass();
    });
    let unShow: (() => void) | undefined;
    let unHide: (() => void) | undefined;
    void (async () => {
      try {
        const initial = await invoke<TipPayload | null>("get_chrome_hover_tip");
        const n = normalizeTip(initial);
        if (n) setTip(n);
      } catch {
        /* noop */
      }
      unShow = await listen<TipPayload>("chrome-hover-tip-show", (ev) => {
        const next = normalizeTip(ev.payload);
        setTip((prev) => (tipPayloadEq(prev, next) ? prev : next));
      });
      unHide = await listen("chrome-hover-tip-hide", () => {
        setTip(null);
        setPlaced(false);
        setHoverHwnd(null);
        genRef.current += 1;
        committedGenRef.current = 0;
      });
    })();
    return () => {
      document.body.classList.remove("is-chrome-hover-tip");
      unsubDark();
      unShow?.();
      unHide?.();
    };
  }, []);

  const placeTip = async (el: HTMLElement, payload: TipPayload, gen: number) => {
    try {
      await fitTipWindow(el, payload);
      if (genRef.current !== gen) return;
      committedGenRef.current = gen;
      setPlaced(true);
    } catch (e) {
      console.error("[ChromeHoverTip] fit", e);
    }
  };

  useLayoutEffect(() => {
    if (!tip) {
      setPlaced(false);
      return;
    }
    const el = boxRef.current;
    if (!el) return;
    const gen = ++genRef.current;
    committedGenRef.current = 0;
    setPlaced(false);

    const run = () => {
      if (genRef.current !== gen) return;
      void placeTip(el, tip, gen).catch(() => undefined);
    };

    // Preview imgs may still be decoding — place after paint; onLoad will re-fit.
    let raf2 = 0;
    const raf1 = window.requestAnimationFrame(() => {
      raf2 = window.requestAnimationFrame(run);
    });
    return () => {
      window.cancelAnimationFrame(raf1);
      window.cancelAnimationFrame(raf2);
    };
  }, [tip]);

  if (!tip) {
    return <div className="chrome-hover-tip-root is-pending" aria-hidden />;
  }

  const previews = tipPreviews(tip);
  const interactive = previews.length > 0;
  const previewH = previewHeightOf(tip);
  const fallbackTitle = tip.lines[0] || "";
  const extraLines = tip.lines.slice(1);
  const iconSrc = tip.iconPngBase64
    ? `data:image/png;base64,${tip.iconPngBase64}`
    : null;

  async function onCloseWindow(hwnd: number) {
    const current = tipRef.current;
    try {
      await invoke("close_window_hwnd", { hwnd });
    } catch (e) {
      console.error("[ChromeHoverTip] close", e);
    }
    const remaining = current ? tipPreviews(current).filter((p) => p.hwnd !== hwnd) : [];
    if (!remaining.length) {
      try {
        await invoke("close_chrome_hover_tip");
      } catch {
        /* noop */
      }
      return;
    }
    setTip((prev) => {
      if (!prev) return null;
      return {
        ...prev,
        previews: remaining,
        imageJpegBase64: remaining[0].jpegBase64,
        hwnd: remaining[0].hwnd,
        lines: remaining[0].title
          ? [remaining[0].title, ...prev.lines.slice(1)]
          : prev.lines,
      };
    });
  }

  async function onActivateWindow(hwnd: number) {
    const current = tipRef.current;
    const itemId = (current?.itemId || "").trim();
    try {
      if (hwnd) {
        await invoke("focus_open_window", { id: `hwnd:${hwnd}` });
      } else if (itemId) {
        await invoke("dock_launch_item", { itemId });
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
      className={`chrome-hover-tip-root${previews.length ? " has-preview" : ""}${interactive ? " is-interactive" : ""}${placed ? "" : " is-pending"}`}
      style={
        {
          ["--dock-preview-h"]: `${previewH}px`,
        } as CSSProperties
      }
    >
      <div
        ref={boxRef}
        className="chrome-hover-tip-box"
        role={interactive ? "group" : "tooltip"}
      >
        {previews.length ? (
          <>
            <div className="chrome-hover-tip-previews">
              {previews.map((pv) => {
                const title = pv.title || fallbackTitle;
                const showClose = hoverHwnd === pv.hwnd;
                return (
                  <div
                    key={pv.hwnd}
                    className="chrome-hover-tip-preview-card"
                    role="button"
                    onPointerMove={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      if (e.clientX >= r.left + r.width * 0.62) {
                        setHoverHwnd(pv.hwnd);
                      } else {
                        setHoverHwnd((h) => (h === pv.hwnd ? null : h));
                      }
                    }}
                    onPointerLeave={() =>
                      setHoverHwnd((h) => (h === pv.hwnd ? null : h))
                    }
                    onClick={(e) => {
                      if ((e.target as HTMLElement).closest?.(".chrome-hover-tip-close")) {
                        return;
                      }
                      e.preventDefault();
                      void onActivateWindow(pv.hwnd);
                    }}
                  >
                    <div className="chrome-hover-tip-title-row">
                      {iconSrc ? (
                        <img
                          className="chrome-hover-tip-icon"
                          src={iconSrc}
                          alt=""
                          draggable={false}
                        />
                      ) : null}
                      <div className="chrome-hover-tip-line is-lead chrome-hover-tip-title">
                        {title}
                      </div>
                      <button
                        type="button"
                        className={`chrome-hover-tip-close${showClose ? " is-visible" : ""}`}
                        aria-label="关闭窗口"
                        tabIndex={-1}
                        onClick={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          void onCloseWindow(pv.hwnd);
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
                    </div>
                    <div className="chrome-hover-tip-preview-wrap">
                      <img
                        key={blobFp(pv.jpegBase64)}
                        className="chrome-hover-tip-preview"
                        src={`data:image/jpeg;base64,${pv.jpegBase64}`}
                        alt=""
                        draggable={false}
                        onLoad={(e) => {
                          const gen = genRef.current;
                          const img = e.currentTarget;
                          const sized = sizeOnePreview(img, previewH, previews.length);
                          img.style.width = `${sized.pw}px`;
                          img.style.height = `${sized.ph}px`;
                          const wrap = img.parentElement;
                          if (wrap) {
                            wrap.style.width = `${sized.pw}px`;
                            wrap.style.height = `${sized.ph}px`;
                          }
                          const card = img.closest(
                            ".chrome-hover-tip-preview-card",
                          ) as HTMLElement | null;
                          if (card) {
                            card.style.width = `${sized.pw}px`;
                            card.style.maxWidth = `${sized.pw}px`;
                          }
                          // Always re-fit after decode — first layout often ran with naturalWidth=0.
                          const el = boxRef.current;
                          const payload = tipRef.current;
                          if (el && payload) {
                            committedGenRef.current = 0;
                            void placeTip(el, payload, gen).catch(() => undefined);
                          }
                        }}
                      />
                    </div>
                  </div>
                );
              })}
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
                {fallbackTitle ? (
                  <div className="chrome-hover-tip-line is-lead chrome-hover-tip-title">
                    {fallbackTitle}
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
