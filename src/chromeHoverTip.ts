import type { PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ChromeHoverTipPlacement = "above" | "below";

export type ChromeHoverTipShowOpts = {
  /** Prefer multi-line tip. */
  lines?: string[];
  /** Single string; split on newlines if `lines` omitted. */
  text?: string;
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
/** Debounce show so a fast flick off the 28px bar never opens a stuck tip. */
let showDebounceTimer: ReturnType<typeof setTimeout> | null = null;
const SHOW_DEBOUNCE_MS = 70;

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
 */
export function installChromeHoverTipGlobalDismiss(): () => void {
  if (globalDismissInstalled || typeof document === "undefined") {
    return () => undefined;
  }
  globalDismissInstalled = true;
  const hide = () => {
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
  return () => {
    globalDismissInstalled = false;
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
  if (!lines.length) {
    await hideChromeHoverTip();
    return;
  }
  tipWanted = true;
  const token = ++tipToken;
  const gap = typeof opts.gap === "number" ? opts.gap : 6;
  const placement: ChromeHoverTipPlacement = opts.placement === "above" ? "above" : "below";

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
    });
    // Slow show finished after a hide: force closed.
    if (!tipWanted || token !== tipToken) {
      if (!tipWanted) {
        await invoke("close_chrome_hover_tip", {}).catch(() => undefined);
      }
    }
  } catch (err) {
    console.error("[chromeHoverTip] show", err);
  }
}

export async function hideChromeHoverTip(): Promise<void> {
  tipWanted = false;
  tipToken += 1;
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
 * Dock icon tip — wait until the hovered icon finishes magnifying, then show
 * once centered above `.dock-hit`. Switching icons cancels the previous tip
 * and restarts the settle timer (no open/close flash storm).
 */
const DOCK_TIP_SETTLE_MS = 180; // matches .dock-hit 160ms width transition
const DOCK_TIP_HIDE_GRACE_MS = 90;

let dockTipEl: HTMLElement | null = null;
let dockTipText = "";
let dockTipGap = 8;
let dockTipShowTimer: ReturnType<typeof setTimeout> | null = null;
let dockTipHideTimer: ReturnType<typeof setTimeout> | null = null;
let dockTipVisibleFor: HTMLElement | null = null;

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

function showDockTipForEl(el: HTMLElement, text: string, gap: number) {
  const r = measureDockHit(el);
  if (!r) return;
  dockTipVisibleFor = el;
  void showChromeHoverTip({
    text,
    x: r.left + r.width / 2,
    y: r.top,
    placement: "above",
    gap,
    immediate: true,
  });
}

function scheduleDockTipShow(el: HTMLElement, text: string, gap: number) {
  clearDockTipHideTimer();
  clearDockTipShowTimer();
  dockTipEl = el;
  dockTipText = text;
  dockTipGap = gap;

  // Leaving A → entering B: drop current tip while B is still growing.
  if (dockTipVisibleFor && dockTipVisibleFor !== el) {
    dockTipVisibleFor = null;
    void hideChromeHoverTip();
  }

  dockTipShowTimer = setTimeout(() => {
    dockTipShowTimer = null;
    if (dockTipEl !== el) return;
    showDockTipForEl(el, text, gap);
    // One late re-anchor after HWND recenter / last fan frame (no continuous follow).
    window.setTimeout(() => {
      if (dockTipEl === el && dockTipVisibleFor === el) {
        showDockTipForEl(el, text, gap);
      }
    }, 40);
  }, DOCK_TIP_SETTLE_MS);
}

function scheduleDockTipHide() {
  clearDockTipShowTimer();
  dockTipEl = null;
  dockTipText = "";
  clearDockTipHideTimer();
  dockTipHideTimer = setTimeout(() => {
    dockTipHideTimer = null;
    // Enter on a sibling may have claimed the tip already.
    if (dockTipEl) return;
    dockTipVisibleFor = null;
    void hideChromeHoverTip();
  }, DOCK_TIP_HIDE_GRACE_MS);
}

export function dockIconTipPointerProps(
  text: string | null | undefined,
  opts?: { gap?: number },
): {
  onPointerEnter?: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerLeave?: (e: ReactPointerEvent<HTMLElement>) => void;
} {
  const tip = (text ?? "").trim();
  if (!tip) return {};
  const gap = typeof opts?.gap === "number" ? opts.gap : 8;
  return {
    onPointerEnter: (e) => {
      scheduleDockTipShow(e.currentTarget, tip, gap);
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
        scheduleDockTipHide();
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
