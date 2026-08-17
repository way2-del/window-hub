import type { PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ChromeHoverTipPlacement = "above" | "below";

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
  return [
    lines.join("\n"),
    blobFp(image),
    blobFp(icon),
    opts.hwnd != null && Number.isFinite(opts.hwnd) ? Math.trunc(opts.hwnd) : "",
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
        const paintImage = (pending.imageJpegBase64 || "").trim() || undefined;
        const paintIcon = (pending.iconPngBase64 || "").trim() || undefined;
        if (!paintLines.length && !paintImage) {
          await hideChromeHoverTip();
          return;
        }

        const gap = typeof pending.gap === "number" ? pending.gap : 6;
        const placement: ChromeHoverTipPlacement =
          pending.placement === "above" ? "above" : "below";
        const sig = tipPaintSig(paintLines, paintImage, paintIcon, {
          ...pending,
          gap,
          placement,
        });
        if (sig === lastTipPaintSig) {
          continue;
        }

        const wantInteractive = Boolean(
          paintImage &&
            pending.hwnd != null &&
            Number.isFinite(pending.hwnd) &&
            pending.hwnd !== 0,
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
              pending.hwnd != null && Number.isFinite(pending.hwnd)
                ? Math.trunc(pending.hwnd)
                : null,
            itemId: (pending.itemId || "").trim() || null,
            iconPngBase64: paintIcon ?? null,
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
          (pending.imageJpegBase64 || "").trim() &&
            pending.hwnd != null &&
            Number.isFinite(pending.hwnd) &&
            pending.hwnd !== 0,
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

type DockPreviewCacheEntry = {
  jpegBase64: string;
  title: string;
  hwnd?: number;
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
  jpegBase64: string,
  title: string,
  hwnd?: number,
) {
  dockPreviewCache.set(itemId, {
    jpegBase64,
    title,
    hwnd,
    at: performance.now(),
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
  }>("dock-preview-ready", (ev) => {
    const itemId = (ev.payload?.itemId || "").trim();
    const jpeg = (ev.payload?.jpegBase64 || "").trim();
    if (!itemId || !jpeg) return;
    const title = (ev.payload?.title || "").trim();
    const hwnd =
      typeof ev.payload?.hwnd === "number" && Number.isFinite(ev.payload.hwnd)
        ? ev.payload.hwnd
        : undefined;
    writePreviewCache(itemId, jpeg, title || itemId, hwnd);
    if (dockPreviewActiveId !== itemId || !dockTipVisibleFor) return;
    const fp = blobFp(jpeg);
    if (fp === dockPreviewShownJpegFp) return;
    dockPreviewShownJpegFp = fp;
    const el = dockTipVisibleFor;
    const r = measureDockHit(el);
    if (!r) return;
    // Debounced update path (immediate + already painted → TIP_UPDATE_DEBOUNCE_MS).
    void showChromeHoverTip({
      text: title || dockPreviewActiveText,
      imageJpegBase64: jpeg,
      iconPngBase64: dockPreviewActiveIcon,
      hwnd,
      itemId,
      x: r.left + r.width / 2,
      y: r.top,
      placement: "above",
      gap: dockPreviewActiveGap,
      immediate: true,
    });
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
) {
  const r = measureDockHit(el);
  if (!r) return;
  dockTipVisibleFor = el;
  const x = r.left + r.width / 2;
  const y = r.top;
  const previewId = (windowPreviewItemId || "").trim();
  const icon = (iconPngBase64 || "").trim() || undefined;

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
  dockPreviewShownJpegFp = "";

  const soft = readPreviewCache(previewId);
  if (soft) {
    dockPreviewShownJpegFp = blobFp(soft.jpegBase64);
    void showChromeHoverTip({
      text: soft.title || text,
      imageJpegBase64: soft.jpegBase64,
      iconPngBase64: icon,
      hwnd: soft.hwnd,
      itemId: previewId,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
  } else {
    // Title + icon immediately; thumbnail arrives via cache / dock-preview-ready.
    void showChromeHoverTip({
      text,
      iconPngBase64: icon,
      itemId: previewId,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
  }

  // Cache-only backend read (prioritizes background refresh) — never waits on capture.
  void (async () => {
    try {
      const prev = await invoke<{
        jpegBase64?: string;
        title?: string;
        hwnd?: number;
      } | null>("dock_capture_window_preview", { itemId: previewId });
      if (gen !== dockPreviewGen || dockTipVisibleFor !== el) return;
      const jpeg = (prev?.jpegBase64 || "").trim();
      if (!jpeg) return;
      const title = (prev?.title || "").trim() || text;
      const hwnd =
        typeof prev?.hwnd === "number" && Number.isFinite(prev.hwnd) ? prev.hwnd : undefined;
      writePreviewCache(previewId, jpeg, title, hwnd);
      const fp = blobFp(jpeg);
      if (fp === dockPreviewShownJpegFp) return;
      dockPreviewShownJpegFp = fp;
      const r2 = measureDockHit(el);
      if (!r2 || dockTipVisibleFor !== el) return;
      void showChromeHoverTip({
        text: title,
        imageJpegBase64: jpeg,
        iconPngBase64: icon,
        hwnd,
        itemId: previewId,
        x: r2.left + r2.width / 2,
        y: r2.top,
        placement: "above",
        gap,
        immediate: true,
      });
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
    showDockTipForEl(el, text, gap, windowPreviewItemId, iconPngBase64);
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
  },
): {
  onPointerEnter?: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerLeave?: (e: ReactPointerEvent<HTMLElement>) => void;
} {
  const tip = (text ?? "").trim();
  if (!tip) return {};
  const gap = typeof opts?.gap === "number" ? opts.gap : 8;
  const windowPreviewItemId = opts?.windowPreviewItemId ?? null;
  const settleMs = opts?.settleMs;
  const iconPngBase64 = opts?.iconPngBase64 ?? null;
  return {
    onPointerEnter: (e) => {
      scheduleDockTipShow(
        e.currentTarget,
        tip,
        gap,
        windowPreviewItemId,
        settleMs,
        iconPngBase64,
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
