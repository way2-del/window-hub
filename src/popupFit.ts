import { LogicalPosition, LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";

export type PopupFitOptions = {
  /** Fixed content width (logical px). */
  width: number;
  /** Root content selector to measure. */
  selector: string;
  minHeight?: number;
  maxHeight?: number;
  /**
   * Keep this logical Y as the bottom edge after resize
   * (e.g. dock menus that open upward).
   */
  pinBottom?: number | null;
};

function clampHeight(measured: number, minH: number, maxH: number) {
  return Math.min(maxH, Math.max(minH, measured));
}

/** Measure natural content height without resizing the window. */
export function measurePopupContent(opts: PopupFitOptions): { width: number; height: number } {
  const el = document.querySelector(opts.selector) as HTMLElement | null;
  const minH = opts.minHeight ?? 40;
  const maxH = opts.maxHeight ?? 720;
  const width = opts.width;
  if (!el) {
    return { width, height: minH };
  }

  const prevHeight = el.style.height;
  const prevMinHeight = el.style.minHeight;
  el.style.height = "auto";
  el.style.minHeight = "0";
  el.style.width = `${opts.width}px`;
  const measured = Math.ceil(el.scrollHeight || el.getBoundingClientRect().height);
  el.style.height = prevHeight;
  el.style.minHeight = prevMinHeight;

  return { width, height: clampHeight(measured, minH, maxH) };
}

/** Measure natural content height and resize the current popup window to fit. */
export async function fitPopupToContent(opts: PopupFitOptions): Promise<{ width: number; height: number }> {
  const { width, height } = measurePopupContent(opts);
  const win = getCurrentWindow();
  await win.setSize(new LogicalSize(width, height));

  if (opts.pinBottom != null && Number.isFinite(opts.pinBottom)) {
    const [factor, pos] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    const x = pos.x / factor;
    const y = opts.pinBottom - height;
    await win.setPosition(new LogicalPosition(x, y));
  }

  return { width, height };
}

/**
 * Dock-like open: measure while hidden, show collapsed, then ease-out expand
 * (~180ms / 15 frames). `down` keeps top edge; `up` pins bottom edge.
 */
export async function slideRevealPopup(
  opts: PopupFitOptions & { direction: "down" | "up" },
): Promise<{ width: number; height: number }> {
  const { width, height } = measurePopupContent(opts);
  const win = getCurrentWindow();
  const [factor, pos] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const x = pos.x / factor;
  const topY = pos.y / factor;
  const pinBottom =
    opts.direction === "up" && opts.pinBottom != null && Number.isFinite(opts.pinBottom)
      ? opts.pinBottom
      : null;

  const startH = 2;
  await win.setSize(new LogicalSize(width, startH));
  if (pinBottom != null) {
    await win.setPosition(new LogicalPosition(x, pinBottom - startH));
  } else {
    await win.setPosition(new LogicalPosition(x, topY));
  }
  await win.show();

  const FRAMES = 15;
  const FRAME_MS = 12;
  const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;

  for (let i = 1; i <= FRAMES; i++) {
    const e = easeOutCubic(i / FRAMES);
    const h = Math.max(startH, Math.round(height * e));
    // Fire without awaiting each IPC — keeps cadence closer to dock's 12ms ticks.
    void win.setSize(new LogicalSize(width, h));
    if (pinBottom != null) {
      void win.setPosition(new LogicalPosition(x, pinBottom - h));
    }
    await new Promise<void>((r) => window.setTimeout(r, FRAME_MS));
  }

  await win.setSize(new LogicalSize(width, height));
  if (pinBottom != null) {
    await win.setPosition(new LogicalPosition(x, pinBottom - height));
  } else {
    await win.setPosition(new LogicalPosition(x, topY));
  }
  await win.setFocus();
  return { width, height };
}

/** Run fit on next frames so layout/fonts settle. */
export function schedulePopupFit(opts: PopupFitOptions, times = [0, 50, 160]) {
  for (const ms of times) {
    window.setTimeout(() => {
      void fitPopupToContent(opts).catch(() => undefined);
    }, ms);
  }
}
