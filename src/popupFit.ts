import { LogicalPosition, LogicalSize, currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

/** Chrome popups (tray / Wi‑Fi / control center): never taller than this fraction of the monitor. */
export const POPUP_MAX_SCREEN_FRACTION = 2 / 3;

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

/** Logical px cap = fraction of the current monitor work height (fallback: availHeight). */
export async function resolvePopupMaxHeight(
  fraction = POPUP_MAX_SCREEN_FRACTION,
): Promise<number> {
  try {
    const [monitor, factor] = await Promise.all([
      currentMonitor(),
      getCurrentWindow().scaleFactor(),
    ]);
    if (monitor) {
      const logical = monitor.size.height / factor;
      return Math.max(160, Math.floor(logical * fraction));
    }
  } catch {
    /* fall through */
  }
  const avail =
    typeof window !== "undefined" && window.screen?.availHeight
      ? window.screen.availHeight
      : 900;
  return Math.max(160, Math.floor(avail * fraction));
}

/** Effective max: caller cap ∩ screen fraction ∩ space left on the monitor. */
export async function effectivePopupMaxHeight(
  explicit?: number,
  opts?: Pick<PopupFitOptions, "pinBottom">,
): Promise<number> {
  const screenCap = await resolvePopupMaxHeight();
  let cap =
    explicit == null || !Number.isFinite(explicit)
      ? screenCap
      : Math.min(explicit, screenCap);

  // Clamp to remaining work area so a tall list under the island scrolls
  // instead of growing past the bottom edge with overflow:hidden.
  try {
    const win = getCurrentWindow();
    const [monitor, factor, pos] = await Promise.all([
      currentMonitor(),
      win.scaleFactor(),
      win.outerPosition(),
    ]);
    if (monitor) {
      const monTop = monitor.position.y / factor;
      const monBottom = monTop + monitor.size.height / factor;
      const margin = 10;
      const space =
        opts?.pinBottom != null && Number.isFinite(opts.pinBottom)
          ? Math.floor(opts.pinBottom - monTop - margin)
          : Math.floor(monBottom - pos.y / factor - margin);
      if (space > 80) {
        cap = Math.min(cap, space);
      }
    }
  } catch {
    /* keep screenCap */
  }
  return cap;
}

export type PopupMeasure = {
  width: number;
  /** Height after min/max clamp (window size). */
  height: number;
  /** Unclamped content height. */
  natural: number;
  /** True when content exceeds the screen/max cap — shell may scroll. */
  scrollable: boolean;
};

/** Extra logical px so DPI / HWND rounding never clips a short menu into a phantom scrollbar. */
const FIT_HEIGHT_SLACK = 2;

/** Measure natural content height without resizing the window. */
export function measurePopupContent(opts: PopupFitOptions): PopupMeasure {
  const el = document.querySelector(opts.selector) as HTMLElement | null;
  const minH = opts.minHeight ?? 40;
  const maxH = opts.maxHeight ?? 720;
  const width = opts.width;
  if (!el) {
    return { width, height: minH, natural: minH, scrollable: false };
  }

  const prevHeight = el.style.height;
  const prevMinHeight = el.style.minHeight;
  const prevMaxHeight = el.style.maxHeight;
  const prevOverflow = el.style.overflow;
  el.style.height = "auto";
  el.style.minHeight = "0";
  el.style.maxHeight = "none";
  el.style.overflow = "visible";
  el.style.width = `${opts.width}px`;
  const natural = Math.ceil(
    Math.max(el.scrollHeight, el.getBoundingClientRect().height),
  );
  el.style.height = prevHeight;
  el.style.minHeight = prevMinHeight;
  el.style.maxHeight = prevMaxHeight;
  el.style.overflow = prevOverflow;

  const height = clampHeight(natural, minH, maxH);
  return { width, height, natural, scrollable: natural > maxH };
}

function syncShellScrollable(selector: string, scrollable: boolean) {
  const el = document.querySelector(selector);
  if (!el) return;
  el.classList.toggle("is-scrollable", scrollable);
}

/** Measure natural content height and resize the current popup window to fit. */
export async function fitPopupToContent(
  opts: PopupFitOptions,
): Promise<{ width: number; height: number; scrollable: boolean }> {
  const maxHeight = await effectivePopupMaxHeight(opts.maxHeight, opts);
  const { width, height, scrollable } = measurePopupContent({ ...opts, maxHeight });
  syncShellScrollable(opts.selector, scrollable);
  // Short menus: slight slack + overflow:hidden (via CSS). Capped menus keep exact max.
  const sizeH = scrollable ? height : height + FIT_HEIGHT_SLACK;
  const win = getCurrentWindow();
  await win.setSize(new LogicalSize(width, sizeH));

  if (opts.pinBottom != null && Number.isFinite(opts.pinBottom)) {
    const [factor, pos] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
    const x = pos.x / factor;
    const y = opts.pinBottom - sizeH;
    await win.setPosition(new LogicalPosition(x, y));
  }

  return { width, height: sizeH, scrollable };
}

/**
 * Dock-like open: measure while hidden, show collapsed, then ease-out expand
 * (~180ms / 15 frames). `down` keeps top edge; `up` pins bottom edge.
 */
export async function slideRevealPopup(
  opts: PopupFitOptions & { direction: "down" | "up" },
): Promise<{ width: number; height: number; scrollable: boolean }> {
  const maxHeight = await effectivePopupMaxHeight(opts.maxHeight, opts);
  const { width, height, scrollable } = measurePopupContent({ ...opts, maxHeight });
  syncShellScrollable(opts.selector, scrollable);
  const sizeH = scrollable ? height : height + FIT_HEIGHT_SLACK;
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
    const h = Math.max(startH, Math.round(sizeH * e));
    // Fire without awaiting each IPC — keeps cadence closer to dock's 12ms ticks.
    void win.setSize(new LogicalSize(width, h));
    if (pinBottom != null) {
      void win.setPosition(new LogicalPosition(x, pinBottom - h));
    }
    await new Promise<void>((r) => window.setTimeout(r, FRAME_MS));
  }

  await win.setSize(new LogicalSize(width, sizeH));
  if (pinBottom != null) {
    await win.setPosition(new LogicalPosition(x, pinBottom - sizeH));
  } else {
    await win.setPosition(new LogicalPosition(x, topY));
  }
  await win.setFocus();
  return { width, height: sizeH, scrollable };
}

/** Run fit on next frames so layout/fonts settle. */
export function schedulePopupFit(opts: PopupFitOptions, times = [0, 50, 160]) {
  for (const ms of times) {
    window.setTimeout(() => {
      void fitPopupToContent(opts).catch(() => undefined);
    }, ms);
  }
}
