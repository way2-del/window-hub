import type { PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ChromeHoverTipShowOpts = {
  /** Prefer multi-line tip. */
  lines?: string[];
  /** Single string; split on newlines if `lines` omitted. */
  text?: string;
  /** Viewport coords inside the calling window (logical CSS px). */
  x: number;
  y: number;
  /** Extra gap below anchor (default 6). */
  gap?: number;
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

  clearShowDebounce();
  await new Promise<void>((resolve) => {
    showDebounceTimer = setTimeout(() => {
      showDebounceTimer = null;
      resolve();
    }, SHOW_DEBOUNCE_MS);
  });
  if (!tipWanted || token !== tipToken) return;

  try {
    const win = getCurrentWindow();
    const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    if (!tipWanted || token !== tipToken) return;
    await invoke("show_chrome_hover_tip", {
      lines,
      x: outer.x / factor + opts.x,
      y: outer.y / factor + opts.y + gap,
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
export function hostTipPointerProps(text: string | null | undefined): {
  onPointerEnter?: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerLeave?: () => void;
} {
  const tip = (text ?? "").trim();
  if (!tip) return {};
  return {
    onPointerEnter: (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      void showChromeHoverTip({
        text: tip,
        x: r.left + r.width / 2,
        y: r.bottom,
      });
    },
    onPointerLeave: () => {
      void hideChromeHoverTip();
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
