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
/** Debounce show so a fast flick off the 28px bar never opens a stuck tip. */
let showDebounceTimer: ReturnType<typeof setTimeout> | null = null;
const SHOW_DEBOUNCE_MS = 70;

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

function clearShowDebounce() {
  if (showDebounceTimer != null) {
    clearTimeout(showDebounceTimer);
    showDebounceTimer = null;
  }
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
 */
export async function showChromeHoverTip(opts: ChromeHoverTipShowOpts): Promise<void> {
  const lines = normalizeLines(opts);
  const imageJpegBase64 = (opts.imageJpegBase64 || "").trim() || undefined;
  if (!lines.length && !imageJpegBase64) {
    await hideChromeHoverTip();
    return;
  }
  tipWanted = true;
  const token = ++tipToken;
  const gap = typeof opts.gap === "number" ? opts.gap : 6;
  const placement: ChromeHoverTipPlacement = opts.placement === "above" ? "above" : "below";
  const wantInteractive = Boolean(
    imageJpegBase64 && opts.hwnd != null && Number.isFinite(opts.hwnd) && opts.hwnd !== 0,
  );

  clearShowDebounce();
  if (!opts.immediate) {
    await new Promise<void>((resolve) => {
      showDebounceTimer = setTimeout(() => {
        showDebounceTimer = null;
        resolve();
      }, SHOW_DEBOUNCE_MS);
    });
    if (!tipWanted || token !== tipToken) return;
  }

  try {
    const win = getCurrentWindow();
    const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    if (!tipWanted || token !== tipToken) return;
    const screenX = outer.x / factor + opts.x;
    // below: tip top = anchor + gap; above: tip bottom = anchor - gap (FE fits with height).
    const screenY =
      placement === "above"
        ? outer.y / factor + opts.y - gap
        : outer.y / factor + opts.y + gap;
    await invoke("show_chrome_hover_tip", {
      lines,
      x: screenX,
      y: screenY,
      placement,
      imageJpegBase64: imageJpegBase64 ?? null,
      hwnd: opts.hwnd != null && Number.isFinite(opts.hwnd) ? Math.trunc(opts.hwnd) : null,
      itemId: (opts.itemId || "").trim() || null,
    });
    if (!tipWanted || token !== tipToken) {
      if (!tipWanted) {
        await invoke("close_chrome_hover_tip", {}).catch(() => undefined);
      }
      return;
    }
    // Only arm — progressive title-first must not clear the session flag.
    if (wantInteractive) {
      interactivePreviewTipLive = true;
    }
  } catch (err) {
    console.error("[chromeHoverTip] show", err);
  }
}

export async function hideChromeHoverTip(): Promise<void> {
  tipWanted = false;
  tipToken += 1;
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
const DOCK_PREVIEW_CACHE_TTL_MS = 10 * 60 * 1000;

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
) {
  const r = measureDockHit(el);
  if (!r) return;
  dockTipVisibleFor = el;
  const x = r.left + r.width / 2;
  const y = r.top;
  const previewId = (windowPreviewItemId || "").trim();

  if (!previewId) {
    void showChromeHoverTip({
      text,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
    return;
  }

  // Arm keep before any tip paint — pointer must leave Dock HWND to reach tip.
  beginDockPreviewSession();

  const gen = ++dockPreviewGen;
  const cached = readPreviewCache(previewId);
  if (cached) {
    void showChromeHoverTip({
      text: cached.title || text,
      imageJpegBase64: cached.jpegBase64,
      hwnd: cached.hwnd,
      itemId: previewId,
      x,
      y,
      placement: "above",
      gap,
      immediate: true,
    });
    return;
  }

  // Progressive: title first (feels instant), then swap in thumbnail when ready.
  void showChromeHoverTip({
    text,
    itemId: previewId,
    x,
    y,
    placement: "above",
    gap,
    immediate: true,
  });

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
      dockPreviewCache.set(previewId, {
        jpegBase64: jpeg,
        title,
        hwnd,
        at: performance.now(),
      });
      const r2 = measureDockHit(el);
      if (!r2 || dockTipVisibleFor !== el) return;
      void showChromeHoverTip({
        text: title,
        imageJpegBase64: jpeg,
        hwnd,
        itemId: previewId,
        x: r2.left + r2.width / 2,
        y: r2.top,
        placement: "above",
        gap,
        immediate: true,
      });
    } catch {
      /* keep text tip */
    }
  })();
}

function scheduleDockTipShow(
  el: HTMLElement,
  text: string,
  gap: number,
  windowPreviewItemId?: string | null,
  settleMs?: number,
) {
  clearDockTipHideTimer();
  clearDockTipShowTimer();
  dockTipEl = el;

  // Switching icons: keep tip hidden until settle completes — do not hide/show
  // on every enter (that was the flash when skimming the bar).
  if (dockTipVisibleFor && dockTipVisibleFor !== el) {
    dockTipVisibleFor = null;
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
    showDockTipForEl(el, text, gap, windowPreviewItemId);
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
      dockPreviewGen += 1;
    }, 420);
    return;
  }
  dockTipHideTimer = setTimeout(() => {
    dockTipHideTimer = null;
    // Enter on a sibling may have claimed the tip already.
    if (dockTipEl) return;
    dockTipVisibleFor = null;
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
  return {
    onPointerEnter: (e) => {
      scheduleDockTipShow(e.currentTarget, tip, gap, windowPreviewItemId, settleMs);
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
