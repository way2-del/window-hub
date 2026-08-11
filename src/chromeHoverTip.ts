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

/** Bumped on every show/hide so in-flight show cannot revive a dismissed tip. */
let tipEpoch = 0;
/** Last intended visibility (hide wins over a slow show). */
let tipWanted = false;
let globalDismissInstalled = false;

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
  const onDocLeave = (ev: MouseEvent) => {
    const to = ev.relatedTarget as Node | null;
    if (to && document.documentElement.contains(to)) return;
    hide();
  };
  document.documentElement.addEventListener("mouseleave", onDocLeave);
  window.addEventListener("blur", hide);
  return () => {
    globalDismissInstalled = false;
    document.documentElement.removeEventListener("mouseleave", onDocLeave);
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
  const epoch = ++tipEpoch;
  const gap = typeof opts.gap === "number" ? opts.gap : 6;
  try {
    const win = getCurrentWindow();
    const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    if (!tipWanted || epoch !== tipEpoch) return;
    await invoke("show_chrome_hover_tip", {
      lines,
      x: outer.x / factor + opts.x,
      y: outer.y / factor + opts.y + gap,
      epoch,
    });
    // Slow show finished after a hide (or newer show): force closed if no longer wanted.
    if (!tipWanted || epoch !== tipEpoch) {
      if (!tipWanted) {
        await invoke("close_chrome_hover_tip", { epoch }).catch(() => undefined);
      }
    }
  } catch (err) {
    console.error("[chromeHoverTip] show", err);
  }
}

export async function hideChromeHoverTip(): Promise<void> {
  tipWanted = false;
  const epoch = ++tipEpoch;
  try {
    await invoke("close_chrome_hover_tip", { epoch });
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
