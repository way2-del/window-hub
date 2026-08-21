import type { PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ChromeHoverTipPlacement = "above" | "below";

export type ChromeHoverTipPreview = {
  jpegBase64: string;
  title: string;
  hwnd: number;
};

export type ChromeHoverTipShowOpts = {
  /** Prefer multi-line tip. */
  lines?: string[];
  /** Single string; split on newlines if `lines` omitted. */
  text?: string;
  /** Optional live window thumbnail (JPEG base64, no data: prefix). */
  imageJpegBase64?: string;
  /** Dock / app icon (PNG base64, no data: prefix) — title leading glyph. */
  iconPngBase64?: string;
  /** Target HWND for interactive preview close button. */
  hwnd?: number | null;
  /** Multi-instance window thumbnails (side-by-side). */
  previews?: ChromeHoverTipPreview[];
  /** Thumbnail CSS height from Dock prefs (default 160). */
  previewHeightPx?: number;
  /** Dock item id — click preview launches/focuses like the icon. */
  itemId?: string | null;
  /** Viewport coords inside the calling window (logical CSS px). */
  x: number;
  y: number;
  /**
   * `below` (default): `(x,y)` is tip top-center (after gap).
   * `above`: `(x,y)` is tip bottom-center — tip sits above the anchor (Dock).
   */
  placement?: ChromeHoverTipPlacement;
  /** Gap between tip and anchor (default 6). */
  gap?: number;
  /** Skip show debounce (follow / refresh while already hovering). */
  immediate?: boolean;
};

/**
 * Local cancel token only (per webview). Backend owns the real show/hide generation —
 * main/dock/tray must NOT share a JS epoch with Rust or tips permanently stop working.
 */
let tipToken = 0;
/** Last intended visibility (hide wins over a slow show). */
let tipWanted = false;
let globalDismissInstalled = false;
/**
 * Interactive Dock window preview tip is live — do not dismiss when the pointer
 * leaves the Dock HWND (it must cross into the tip window above).
 */
let interactivePreviewTipLive = false;
/** Debounce / coalesce show so skim + progressive paint don't thrash the tip HWND. */
let showDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let showCoalesceOpts: ChromeHoverTipShowOpts | null = null;
let tipPaintInFlight = false;
/** Last successfully invoked tip fingerprint — skip identical re-shows. */
let lastTipPaintSig = "";
const SHOW_DEBOUNCE_MS = 70;
/** Image / progressive updates while tip already open. */
const TIP_UPDATE_DEBOUNCE_MS = 120;

export function isInteractiveChromeHoverTipLive(): boolean {
  return interactivePreviewTipLive;
}

function normalizeLines(opts: ChromeHoverTipShowOpts): string[] {
  if (Array.isArray(opts.lines) && opts.lines.length) {
    return opts.lines.map((l) => String(l ?? "").trim()).filter(Boolean).slice(0, 8);
  }
  const text = (opts.text ?? "").trim();
  if (!text) return [];
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean)
    .slice(0, 8);
}

/** Cheap stable id for large base64 blobs (avoid full-string compare thrash). */
function blobFp(s: string | undefined): string {
  if (!s) return "";
  const n = s.length;
  if (n <= 64) return `${n}:${s}`;
  return `${n}:${s.slice(0, 32)}:${s.slice(-32)}`;
}

function tipPaintSig(
  lines: string[],
  image: string | undefined,
  icon: string | undefined,
  opts: ChromeHoverTipShowOpts,
): string {
  const previewsSig = (opts.previews || [])
    .map((p) => `${p.hwnd}:${blobFp(p.jpegBase64)}:${(p.title || "").slice(0, 24)}`)
    .join(";");
  return [
    lines.join("\n"),
    blobFp(image),
    blobFp(icon),
    previewsSig,
    opts.hwnd != null && Number.isFinite(opts.hwnd) ? Math.trunc(opts.hwnd) : "",
    typeof opts.previewHeightPx === "number" ? Math.round(opts.previewHeightPx) : "",
    (opts.itemId || "").trim(),
    opts.placement === "above" ? "above" : "below",
    Math.round(opts.x),
    Math.round(opts.y),
    typeof opts.gap === "number" ? opts.gap : 6,
  ].join("|");
}

function clearShowDebounceTimerOnly() {
  if (showDebounceTimer != null) {
    clearTimeout(showDebounceTimer);
    showDebounceTimer = null;
  }
}

function clearShowDebounce() {
  clearShowDebounceTimerOnly();
  showCoalesceOpts = null;
}

/**
 * When the pointer leaves the Host webview (quick flick off the 28px bar),
 * element pointerleave can miss — dismiss tip at the document edge / blur.
 * Interactive Dock previews are excluded: the tip is a separate HWND above the
 * dock, so leaving the dock is required to reach the close button.
 */
export function installChromeHoverTipGlobalDismiss(): () => void {
  if (globalDismissInstalled || typeof document === "undefined") {
    return () => undefined;
  }
  globalDismissInstalled = true;
  const hide = () => {
    if (interactivePreviewTipLive) return;
    void hideChromeHoverTip();
  };
  /** relatedTarget null / outside document = left the webview HWND. */
  const onMouseOut = (ev: MouseEvent) => {
    const to = ev.relatedTarget as Node | null;
    if (to && document.documentElement.contains(to)) return;
    hide();
  };
  const onDocLeave = (ev: MouseEvent) => {
    const to = ev.relatedTarget as Node | null;
    if (to && document.documentElement.contains(to)) return;
    hide();
  };
  document.documentElement.addEventListener("mouseleave", onDocLeave);
  document.addEventListener("mouseout", onMouseOut, true);
  window.addEventListener("pointerleave", hide);
  window.addEventListener("blur", hide);
  let unHide: (() => void) | undefined;
  void listen("chrome-hover-tip-hide", () => {
    interactivePreviewTipLive = false;
  }).then((fn) => {
    unHide = fn;
  });
  return () => {
    globalDismissInstalled = false;
    unHide?.();
    document.documentElement.removeEventListener("mouseleave", onDocLeave);
    document.removeEventListener("mouseout", onMouseOut, true);
    window.removeEventListener("pointerleave", hide);
    window.removeEventListener("blur", hide);
  };
}

/**
 * System chrome hover tip — same Mica + glass wash as plugin popup.
 * Safe to call from main / dock / tray / tip windows (screen coords derived from caller).
 * Coalesces + fingerprints so preview / tip do not repeatedly resize the HWND.
 */
export async function showChromeHoverTip(opts: ChromeHoverTipShowOpts): Promise<void> {
  const lines = normalizeLines(opts);
  const imageJpegBase64 = (opts.imageJpegBase64 || "").trim() || undefined;
  if (!lines.length && !imageJpegBase64) {
    await hideChromeHoverTip();
    return;
  }

  tipWanted = true;
  // Do not bump tipToken here — hide() owns cancellation. Bumping on every paint
  // cancelled in-flight first paint when soft-cache + backend raced.
  const epoch = tipToken;
  showCoalesceOpts = { ...opts, lines, imageJpegBase64, text: undefined };

  const wantInteractiveNow = Boolean(
    imageJpegBase64 &&
      opts.hwnd != null &&
      Number.isFinite(opts.hwnd) &&
      opts.hwnd !== 0,
  );

  // In-flight paint will drain the latest coalesce — don't start a second runner.
  if (tipPaintInFlight) return;

  // Interactive preview must flush immediately — debounce left the HWND click-through
  // / unfitted, so leave-watch hid it before the user could reach the close button.
  const delay =
    wantInteractiveNow || (opts.immediate && !lastTipPaintSig)
      ? 0
      : opts.immediate
        ? TIP_UPDATE_DEBOUNCE_MS
        : SHOW_DEBOUNCE_MS;

  clearShowDebounceTimerOnly();

  const run = async () => {
    tipPaintInFlight = true;
    showDebounceTimer = null;
    try {
      // Drain coalesced paints — a newer show may arrive while invoke is in flight.
      while (tipWanted && epoch === tipToken) {
        const pending = showCoalesceOpts;
        showCoalesceOpts = null;
        if (!pending) return;

        const paintLines = normalizeLines(pending);
        const paintPreviews = (pending.previews || [])
          .map((p) => ({
            jpegBase64: (p.jpegBase64 || "").trim(),
            title: String(p.title || "").trim(),
            hwnd:
              typeof p.hwnd === "number" && Number.isFinite(p.hwnd) && p.hwnd !== 0
                ? Math.trunc(p.hwnd)
                : 0,
          }))
          .filter((p) => p.jpegBase64 && p.hwnd)
          .slice(0, 8);
        const paintImage =
          paintPreviews[0]?.jpegBase64 ||
          (pending.imageJpegBase64 || "").trim() ||
          undefined;
        const paintIcon = (pending.iconPngBase64 || "").trim() || undefined;
        if (!paintLines.length && !paintImage && !paintPreviews.length) {
          await hideChromeHoverTip();
          return;
        }

        const gap = typeof pending.gap === "number" ? pending.gap : 6;
        const placement: ChromeHoverTipPlacement =
          pending.placement === "above" ? "above" : "below";
        const sig = tipPaintSig(paintLines, paintImage, paintIcon, {
          ...pending,
          previews: paintPreviews,
          gap,
          placement,
        });
        if (sig === lastTipPaintSig) {
          continue;
        }

        const wantInteractive = Boolean(
          paintPreviews.length > 0 ||
            (paintImage &&
              pending.hwnd != null &&
              Number.isFinite(pending.hwnd) &&
              pending.hwnd !== 0),
        );

        try {
          const win = getCurrentWindow();
          const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
          if (!tipWanted || epoch !== tipToken) return;
          if (showCoalesceOpts) continue;
          const screenX = outer.x / factor + pending.x;
          const screenY =
            placement === "above"
              ? outer.y / factor + pending.y - gap
              : outer.y / factor + pending.y + gap;
          await invoke("show_chrome_hover_tip", {
            lines: paintLines,
            x: screenX,
            y: screenY,
            placement,
            imageJpegBase64: paintImage ?? null,
            hwnd:
              paintPreviews[0]?.hwnd ??
              (pending.hwnd != null && Number.isFinite(pending.hwnd)
                ? Math.trunc(pending.hwnd)
                : null),
            itemId: (pending.itemId || "").trim() || null,
            iconPngBase64: paintIcon ?? null,
            previews: paintPreviews.length
              ? paintPreviews.map((p) => ({
                  jpegBase64: p.jpegBase64,
                  title: p.title,
                  hwnd: p.hwnd,
                }))
              : null,
            previewHeightPx:
              typeof pending.previewHeightPx === "number" &&
              Number.isFinite(pending.previewHeightPx)
                ? Math.min(320, Math.max(96, Math.round(pending.previewHeightPx)))
                : null,
          });
          if (!tipWanted || epoch !== tipToken) {
            if (!tipWanted) {
              await invoke("close_chrome_hover_tip", {}).catch(() => undefined);
            }
            return;
          }
          if (showCoalesceOpts) continue;
          lastTipPaintSig = sig;
          // Arm as soon as we intend a dock preview session (itemId), not only
          // after interactive paint — pointerleave on Dock must not dismiss.
          if (wantInteractive || (pending.itemId || "").trim()) {
            interactivePreviewTipLive = true;
          }
        } catch (err) {
          console.error("[chromeHoverTip] show", err);
          return;
        }
      }
    } finally {
      tipPaintInFlight = false;
      // A show landed after the loop exited but before finally — schedule drain.
      if (showCoalesceOpts && tipWanted && epoch === tipToken) {
        const pending = showCoalesceOpts;
        const flushInteractive = Boolean(
          (pending.previews && pending.previews.length > 0) ||
            ((pending.imageJpegBase64 || "").trim() &&
              pending.hwnd != null &&
              Number.isFinite(pending.hwnd) &&
              pending.hwnd !== 0),
        );
        clearShowDebounceTimerOnly();
        if (flushInteractive) {
          void showChromeHoverTip(pending);
        } else {
          showDebounceTimer = setTimeout(() => {
            void showChromeHoverTip(showCoalesceOpts ?? pending);
          }, TIP_UPDATE_DEBOUNCE_MS);
        }
      }
    }
  };

  if (delay <= 0) {
    await run();
    return;
  }
  await new Promise<void>((resolve) => {
    showDebounceTimer = setTimeout(() => {
      void run().finally(resolve);
    }, delay);
  });
}

export async function hideChromeHoverTip(): Promise<void> {
  tipWanted = false;
  tipToken += 1;
  lastTipPaintSig = "";
  tipPaintInFlight = false;
  endDockPreviewSession();
  clearShowDebounce();
  try {
    await invoke("close_chrome_hover_tip", {});
  } catch {
    /* noop */
  }
}

/** Drop-in replacement for native `title` on Host chrome controls. */
export function hostTipPointerProps(
  text: string | null | undefined,
  opts?: { placement?: ChromeHoverTipPlacement; gap?: number },
): {
  onPointerEnter?: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerLeave?: () => void;
} {
  const tip = (text ?? "").trim();
  if (!tip) return {};
  const placement = opts?.placement === "above" ? "above" : "below";
  return {
    onPointerEnter: (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      void showChromeHoverTip({
        text: tip,
        x: r.left + r.width / 2,
        y: placement === "above" ? r.top : r.bottom,
        placement,
        gap: opts?.gap,
      });
    },
    onPointerLeave: () => {
      void hideChromeHoverTip();
    },
  };
}

/**
 * Dock icon tip — wait until the pointer settles on an icon, then show once
 * above `.dock-hit`. Fast skimming only restarts the timer (no tip flash).
 */
const DOCK_TIP_SETTLE_MS = 420;
const DOCK_TIP_PREVIEW_SETTLE_DEFAULT_MS = 120;
const DOCK_TIP_HIDE_GRACE_MS = 120;
/** Soft FE mirror of backend cache — hover paints instantly; refresher updates. */
const DOCK_PREVIEW_CACHE_TTL_MS = 30_000;

type DockPreviewWindow = {
  jpegBase64: string;
  title: string;
  hwnd: number;
};

type DockPreviewCacheEntry = {
  windows: DockPreviewWindow[];
  previewHeightPx: number;
  at: number;
};

const dockPreviewCache = new Map<string, DockPreviewCacheEntry>();

let dockTipEl: HTMLElement | null = null;
let dockTipShowTimer: ReturnType<typeof setTimeout> | null = null;
let dockTipHideTimer: ReturnType<typeof setTimeout> | null = null;
let dockTipVisibleFor: HTMLElement | null = null;
/** Bumped to ignore stale capture results. */
let dockPreviewGen = 0;
/** Item id currently shown with preview tip (for background refresh paint). */
let dockPreviewActiveId: string | null = null;
let dockPreviewActiveText = "";
let dockPreviewActiveIcon: string | undefined;
let dockPreviewActiveGap = 8;
let dockPreviewActiveHeight = 160;
let dockPreviewReadyListening = false;
/** Last jpeg fp applied to the open preview tip — skip identical background frames. */
let dockPreviewShownJpegFp = "";

function clearDockTipShowTimer() {
  if (dockTipShowTimer != null) {
    clearTimeout(dockTipShowTimer);
    dockTipShowTimer = null;
  }
}

function clearDockTipHideTimer() {
  if (dockTipHideTimer != null) {
    clearTimeout(dockTipHideTimer);
    dockTipHideTimer = null;
  }
}

function measureDockHit(el: HTMLElement): DOMRect | null {
  const hit = el.querySelector(".dock-hit") as HTMLElement | null;
  const r = (hit ?? el).getBoundingClientRect();
  if (r.width < 1 || r.height < 1) return null;
  return r;
}

/**
 * Tip sits just above the live `.dock-hit` (already grows with magnification).
 * Do not reserve the full HWND headroom — that left a large empty gap above the icon.
 * Fan-arm race is handled by the short re-anchor after settle.
 */
function measureDockTipAnchor(el: HTMLElement): { x: number; y: number } | null {
  const r = measureDockHit(el);
  if (!r) return null;
  return {
    x: r.left + r.width / 2,
    y: r.top,
  };
}

function windowsFp(windows: DockPreviewWindow[]): string {
  return windows
    .map((w) => `${w.hwnd}:${blobFp(w.jpegBase64)}:${(w.title || "").slice(0, 24)}`)
    .join(";");
}

function readPreviewCache(itemId: string): DockPreviewCacheEntry | null {
  const hit = dockPreviewCache.get(itemId);
  if (!hit) return null;
  if (performance.now() - hit.at > DOCK_PREVIEW_CACHE_TTL_MS) {
    dockPreviewCache.delete(itemId);
    return null;
  }
  return hit;
}

function writePreviewCache(
  itemId: string,
  windows: DockPreviewWindow[],
  previewHeightPx: number,
) {
  if (!windows.length) return;
  dockPreviewCache.set(itemId, {
    windows,
    previewHeightPx,
    at: performance.now(),
  });
}

/** Drop FE soft cache (e.g. preview height changed) so width remeasures from fresh frames. */
export function clearDockPreviewSoftCache() {
  dockPreviewCache.clear();
  dockPreviewShownJpegFp = "";
}

function normalizeCaptureWindows(
  raw:
    | {
        windows?: Array<{
          jpegBase64?: string;
          title?: string;
          hwnd?: number;
        }>;
        jpegBase64?: string;
        title?: string;
        hwnd?: number;
        previewHeightPx?: number;
      }
    | null
    | undefined,
  fallbackTitle: string,
): { windows: DockPreviewWindow[]; previewHeightPx: number } | null {
  if (!raw) return null;
  const height =
    typeof raw.previewHeightPx === "number" && Number.isFinite(raw.previewHeightPx)
      ? Math.min(320, Math.max(96, Math.round(raw.previewHeightPx)))
      : dockPreviewActiveHeight;
  let windows: DockPreviewWindow[] = [];
  if (Array.isArray(raw.windows) && raw.windows.length) {
    const parsed: DockPreviewWindow[] = [];
    for (const w of raw.windows) {
      const jpegBase64 = (w.jpegBase64 || "").trim();
      const hwnd =
        typeof w.hwnd === "number" && Number.isFinite(w.hwnd) && w.hwnd !== 0
          ? Math.trunc(w.hwnd)
          : 0;
      if (!jpegBase64 || !hwnd) continue;
      parsed.push({
        jpegBase64,
        title: (w.title || "").trim() || fallbackTitle,
        hwnd,
      });
      if (parsed.length >= 8) break;
    }
    windows = parsed;
  } else {
    const jpeg = (raw.jpegBase64 || "").trim();
    const hwnd =
      typeof raw.hwnd === "number" && Number.isFinite(raw.hwnd) && raw.hwnd !== 0
        ? Math.trunc(raw.hwnd)
        : 0;
    if (jpeg && hwnd) {
      windows = [
        {
          jpegBase64: jpeg,
          title: (raw.title || "").trim() || fallbackTitle,
          hwnd,
        },
      ];
    }
  }
  if (!windows.length) return null;
  return { windows, previewHeightPx: height };
}

function paintDockPreviewTip(
  el: HTMLElement,
  text: string,
  gap: number,
  icon: string | undefined,
  previewId: string,
  windows: DockPreviewWindow[],
  previewHeightPx: number,
  immediate: boolean,
) {
  const anchor = measureDockTipAnchor(el);
  if (!anchor) return;
  const first = windows[0];
  void showChromeHoverTip({
    text: first?.title || text,
    imageJpegBase64: first?.jpegBase64,
    iconPngBase64: icon,
    hwnd: first?.hwnd,
    itemId: previewId,
    previews: windows.map((w) => ({
      jpegBase64: w.jpegBase64,
      title: w.title || text,
      hwnd: w.hwnd,
    })),
    previewHeightPx,
    x: anchor.x,
    y: anchor.y,
    placement: "above",
    gap,
    immediate,
  });
}

function ensureDockPreviewReadyListener() {
  if (dockPreviewReadyListening) return;
  dockPreviewReadyListening = true;
  void listen<{
    itemId?: string;
    jpegBase64?: string;
    title?: string;
    hwnd?: number;
    previewHeightPx?: number;
    windows?: Array<{
      jpegBase64?: string;
      title?: string;
      hwnd?: number;
    }>;
  }>("dock-preview-ready", (ev) => {
    const itemId = (ev.payload?.itemId || "").trim();
    if (!itemId) return;
    const normalized = normalizeCaptureWindows(ev.payload, itemId);
    if (!normalized) return;
    writePreviewCache(itemId, normalized.windows, normalized.previewHeightPx);
    if (dockPreviewActiveId !== itemId || !dockTipVisibleFor) return;
    const fp = windowsFp(normalized.windows);
    if (fp === dockPreviewShownJpegFp) return;
    dockPreviewShownJpegFp = fp;
    dockPreviewActiveHeight = normalized.previewHeightPx;
    paintDockPreviewTip(
      dockTipVisibleFor,
      dockPreviewActiveText,
      dockPreviewActiveGap,
      dockPreviewActiveIcon,
      itemId,
      normalized.windows,
      normalized.previewHeightPx,
      true,
    );
  });
}

function beginDockPreviewSession() {
  interactivePreviewTipLive = true;
  void invoke("dock_set_preview_tip_keep", { keep: true }).catch(() => undefined);
}

function endDockPreviewSession() {
  interactivePreviewTipLive = false;
  void invoke("dock_set_preview_tip_keep", { keep: false }).catch(() => undefined);
}

function showDockTipForEl(
  el: HTMLElement,
  text: string,
  gap: number,
  windowPreviewItemId?: string | null,
  iconPngBase64?: string | null,
  previewHeightPx?: number | null,
) {
  const anchor = measureDockTipAnchor(el);
  if (!anchor) return;
  dockTipVisibleFor = el;
  const { x, y } = anchor;
  const previewId = (windowPreviewItemId || "").trim();
  const icon = (iconPngBase64 || "").trim() || undefined;
  const height =
    typeof previewHeightPx === "number" && Number.isFinite(previewHeightPx)
      ? Math.min(320, Math.max(96, Math.round(previewHeightPx)))
      : 160;

  if (!previewId) {
    dockPreviewActiveId = null;
    void showChromeHoverTip({
      text,
      iconPngBase64: icon,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
    // Fan may arm after settle — re-anchor once without content flash if Y drops.
    window.setTimeout(() => {
      if (dockTipVisibleFor !== el) return;
      const a2 = measureDockTipAnchor(el);
      if (!a2) return;
      if (Math.abs(a2.y - y) < 1 && Math.abs(a2.x - x) < 1) return;
      void showChromeHoverTip({
        text,
        iconPngBase64: icon,
        x: a2.x,
        y: a2.y,
        placement: "above",
        gap,
        immediate: true,
      });
    }, 160);
    return;
  }

  ensureDockPreviewReadyListener();
  // Arm keep before any tip paint — pointer must leave Dock HWND to reach tip.
  beginDockPreviewSession();

  const gen = ++dockPreviewGen;
  dockPreviewActiveId = previewId;
  dockPreviewActiveText = text;
  dockPreviewActiveIcon = icon;
  dockPreviewActiveGap = gap;
  dockPreviewActiveHeight = height;
  dockPreviewShownJpegFp = "";

  const soft = readPreviewCache(previewId);
  if (soft?.windows.length) {
    dockPreviewShownJpegFp = windowsFp(soft.windows);
    // Always use current prefs height — stale soft.previewHeightPx squeezed width after resize.
    paintDockPreviewTip(
      el,
      text,
      gap,
      icon,
      previewId,
      soft.windows,
      height,
      true,
    );
  } else {
    // Title + icon immediately; thumbnail arrives via cache / dock-preview-ready.
    void showChromeHoverTip({
      text,
      iconPngBase64: icon,
      itemId: previewId,
      previewHeightPx: height,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
  }

  // After fan arms / chrome widens, nudge tip above peaking icon (same payload).
  window.setTimeout(() => {
    if (dockTipVisibleFor !== el || gen !== dockPreviewGen) return;
    const a2 = measureDockTipAnchor(el);
    if (!a2) return;
    if (Math.abs(a2.y - y) < 1 && Math.abs(a2.x - x) < 1) return;
    const soft2 = readPreviewCache(previewId);
    if (soft2?.windows.length) {
      paintDockPreviewTip(
        el,
        text,
        gap,
        icon,
        previewId,
        soft2.windows,
        height,
        true,
      );
    } else {
      void showChromeHoverTip({
        text,
        iconPngBase64: icon,
        itemId: previewId,
        previewHeightPx: height,
        x: a2.x,
        y: a2.y,
        placement: "above",
        gap,
        immediate: true,
      });
    }
  }, 160);

  // Cache-only backend read (prioritizes background refresh) — never waits on capture.
  void (async () => {
    try {
      const prev = await invoke<{
        windows?: Array<{
          jpegBase64?: string;
          title?: string;
          hwnd?: number;
        }>;
        jpegBase64?: string;
        title?: string;
        hwnd?: number;
        previewHeightPx?: number;
      } | null>("dock_capture_window_preview", { itemId: previewId });
      if (gen !== dockPreviewGen || dockTipVisibleFor !== el) return;
      const normalized = normalizeCaptureWindows(prev, text);
      if (!normalized) return;
      writePreviewCache(previewId, normalized.windows, normalized.previewHeightPx);
      const fp = windowsFp(normalized.windows);
      if (fp === dockPreviewShownJpegFp) return;
      dockPreviewShownJpegFp = fp;
      if (dockTipVisibleFor !== el) return;
      paintDockPreviewTip(
        el,
        text,
        gap,
        icon,
        previewId,
        normalized.windows,
        normalized.previewHeightPx,
        true,
      );
    } catch {
      /* keep text / soft tip */
    }
  })();
}

function scheduleDockTipShow(
  el: HTMLElement,
  text: string,
  gap: number,
  windowPreviewItemId?: string | null,
  settleMs?: number,
  iconPngBase64?: string | null,
  previewHeightPx?: number | null,
) {
  clearDockTipHideTimer();
  clearDockTipShowTimer();
  dockTipEl = el;

  // Switching icons: keep tip hidden until settle completes — do not hide/show
  // on every enter (that was the flash when skimming the bar).
  if (dockTipVisibleFor && dockTipVisibleFor !== el) {
    dockTipVisibleFor = null;
    dockPreviewActiveId = null;
    dockPreviewGen += 1;
    void hideChromeHoverTip();
  }

  const previewId = (windowPreviewItemId || "").trim();
  const delay = previewId
    ? Math.min(
        2000,
        Math.max(
          0,
          typeof settleMs === "number" && Number.isFinite(settleMs)
            ? settleMs
            : DOCK_TIP_PREVIEW_SETTLE_DEFAULT_MS,
        ),
      )
    : DOCK_TIP_SETTLE_MS;

  dockTipShowTimer = setTimeout(() => {
    dockTipShowTimer = null;
    if (dockTipEl !== el) return;
    showDockTipForEl(el, text, gap, windowPreviewItemId, iconPngBase64, previewHeightPx);
  }, delay);
}

function scheduleDockTipHide(previewActive = false) {
  clearDockTipShowTimer();
  dockTipEl = null;
  clearDockTipHideTimer();
  // Preview tip is interactive in another HWND — FE relatedTarget can't see it.
  // Let backend leave-watch dismiss when the cursor leaves dock + tip hosts.
  if (previewActive) {
    dockTipHideTimer = setTimeout(() => {
      dockTipHideTimer = null;
      if (dockTipEl) return;
      dockTipVisibleFor = null;
      dockPreviewActiveId = null;
      dockPreviewShownJpegFp = "";
      dockPreviewGen += 1;
    }, 420);
    return;
  }
  dockTipHideTimer = setTimeout(() => {
    dockTipHideTimer = null;
    // Enter on a sibling may have claimed the tip already.
    if (dockTipEl) return;
    dockTipVisibleFor = null;
    dockPreviewActiveId = null;
    dockPreviewShownJpegFp = "";
    dockPreviewGen += 1;
    void hideChromeHoverTip();
  }, DOCK_TIP_HIDE_GRACE_MS);
}

export function dockIconTipPointerProps(
  text: string | null | undefined,
  opts?: {
    gap?: number;
    windowPreviewItemId?: string | null;
    /** Override settle delay when showing window preview (ms). */
    settleMs?: number;
    /** Dock tile icon (PNG base64) for preview title row. */
    iconPngBase64?: string | null;
    /** Thumbnail height in CSS px (Dock prefs). */
    previewHeightPx?: number | null;
  },
): {
  onPointerEnter?: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerLeave?: (e: ReactPointerEvent<HTMLElement>) => void;
} {
  const tip = (text ?? "").trim();
  if (!tip) return {};
  const gap = typeof opts?.gap === "number" ? opts.gap : 2;
  const windowPreviewItemId = opts?.windowPreviewItemId ?? null;
  const settleMs = opts?.settleMs;
  const iconPngBase64 = opts?.iconPngBase64 ?? null;
  const previewHeightPx = opts?.previewHeightPx ?? null;
  return {
    onPointerEnter: (e) => {
      scheduleDockTipShow(
        e.currentTarget,
        tip,
        gap,
        windowPreviewItemId,
        settleMs,
        iconPngBase64,
        previewHeightPx,
      );
    },
    onPointerLeave: (e) => {
      const to = e.relatedTarget as Node | null;
      // Moving onto another dock icon — let its enter restart settle; grace hide covers gaps.
      if (to && (to as HTMLElement).closest?.(".dock-item")) {
        if (dockTipEl === e.currentTarget) {
          clearDockTipShowTimer();
          dockTipEl = null;
        }
        return;
      }
      if (dockTipEl === e.currentTarget || dockTipVisibleFor === e.currentTarget) {
        scheduleDockTipHide(Boolean(windowPreviewItemId));
      }
    },
  };
}

/** Show tip under an element (lines or text). */
export function showChromeHoverTipForEl(
  el: HTMLElement,
  content: { lines?: string[]; text?: string },
): void {
  const r = el.getBoundingClientRect();
  void showChromeHoverTip({
    ...content,
    x: r.left + r.width / 2,
    y: r.bottom,
  });
}
