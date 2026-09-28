/** Pure popup placement — keep WxH rect inside a work area; flip when needed. */

export type WorkRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

export type PopupPlacement = {
  x: number;
  y: number;
};

export type FitPopupOriginOpts = {
  /** Inset from work-area edges. Default 8. */
  margin?: number;
  /**
   * When overflowing the bottom, try placing the popup above this Y
   * (typically the trigger top). If omitted, only clamp downward.
   */
  flipAboveY?: number;
  /** Gap used with flipAboveY. Default 8. */
  flipGap?: number;
};

/**
 * Fit a popup's top-left so the WxH rect stays inside `work`.
 * Horizontal: clamp. Vertical: prefer flip above `flipAboveY` when bottom overflows.
 */
export function fitPopupOrigin(
  x: number,
  y: number,
  w: number,
  h: number,
  work: WorkRect,
  opts?: FitPopupOriginOpts,
): PopupPlacement {
  const margin = opts?.margin ?? 8;
  const flipGap = opts?.flipGap ?? 8;
  const width = Math.max(1, w);
  const height = Math.max(1, h);

  const left = work.left + margin;
  const top = work.top + margin;
  const right = work.right - margin;
  const bottom = work.bottom - margin;

  let nextX = x;
  let nextY = y;

  const maxX = Math.max(left, right - width);
  if (nextX + width > right) nextX = maxX;
  if (nextX < left) nextX = left;

  if (nextY + height > bottom) {
    const flipY =
      opts?.flipAboveY != null && Number.isFinite(opts.flipAboveY)
        ? opts.flipAboveY - flipGap - height
        : NaN;
    if (Number.isFinite(flipY) && flipY >= top) {
      nextY = flipY;
    } else {
      nextY = Math.max(top, bottom - height);
    }
  }
  if (nextY < top) nextY = top;

  return { x: nextX, y: nextY };
}

/**
 * Place a popup below a trigger rect; right-align when left-align would overflow.
 * `estimatedH` drives vertical flip decisions before the real size is known.
 */
export function placePopupNearRect(
  trigger: { left: number; top: number; right: number; bottom: number },
  popupW: number,
  popupH: number,
  work: WorkRect,
  gap = 8,
): PopupPlacement {
  let x = trigger.left;
  if (x + popupW > work.right - 8) {
    x = trigger.right - popupW;
  }
  const y = trigger.bottom + gap;
  return fitPopupOrigin(x, y, popupW, popupH, work, {
    margin: 8,
    flipAboveY: trigger.top,
    flipGap: gap,
  });
}
