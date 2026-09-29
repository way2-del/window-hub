/**
 * Bloub SVG mount — framework-free DOM renderer for BotEngine frames.
 * Engine: https://github.com/jeremy-prt/bloub (MIT)
 *
 * `simple` mode (shortcuts): body fill + eye overlays — no <mask>.
 * Mask holes are unreliable in Host shortcuts iframes (WebView2 srcdoc).
 */
import { type Block } from "../vendor/bot/cycles";
import { NOTIF_BLUE } from "../vendor/bot/decor";
import { BotEngine, type BotFrame, type Look } from "../vendor/bot/engine";
import {
  DEFAULT_EXPRESSION,
  EXPRESSION_BY_ID,
  EXPRESSIONS,
  type BotExpression,
  type ExpressionId,
} from "../vendor/bot/expressions";
import { DEMI_VIEWBOX, RAYON } from "../vendor/bot/repere";
import { mixHex, SHAPE_BY_ID } from "../vendor/bot/skins";
import type { StateId } from "../vendor/bot/states";
import type { MontageBlock } from "./sequence";

export { EXPRESSIONS, EXPRESSION_BY_ID, DEFAULT_EXPRESSION, BotEngine };
export type { ExpressionId, StateId, BotExpression, Block, MontageBlock };

const VB = DEMI_VIEWBOX;
const R = RAYON;

export type BloubColors = {
  ink: string;
  paper: string;
};

export type BloubMountOptions = {
  size: number;
  ink?: string;
  paper?: string;
  expression?: ExpressionId | string;
  state?: StateId;
  /** Customiser shape id (cercle / galet / …). */
  shape?: string;
  cycle?: MontageBlock[] | null;
  playing?: boolean;
  follow?: boolean;
  /** Crop viewBox to ±1.15R (bar chip). */
  tight?: boolean;
  /**
   * Draw body + eye overlays (no SVG mask). Required for shortcuts iframe.
   * Popup can keep mask mode for correct edge clipping.
   */
  simple?: boolean;
  className?: string;
};

function uid() {
  return Math.random().toString(36).slice(2, 8);
}

function parseRgb(css: string): { r: number; g: number; b: number } | null {
  const m = css.match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/i);
  if (!m) return null;
  return { r: Number(m[1]), g: Number(m[2]), b: Number(m[3]) };
}

export function paperForInk(inkCss: string): string {
  const rgb = parseRgb(inkCss);
  if (!rgb) return "#ffffff";
  const lum = (0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b) / 255;
  return lum > 0.55 ? "#0a0a0c" : "#ffffff";
}

export function createBloubMount(host: HTMLElement, opts: BloubMountOptions) {
  const id = uid();
  const maskId = `bloub-mask-${id}`;
  let ink = opts.ink ?? "#0a0a0c";
  let paper = opts.paper ?? paperForInk(ink);
  let expressionId: string = opts.expression ?? DEFAULT_EXPRESSION;
  const simple = opts.simple !== false; // default simple — safest
  const half = opts.tight ? Math.ceil(R * 1.15) : VB;

  const expr0 = EXPRESSION_BY_ID.get(expressionId) ?? null;
  // Match upstream BloubBot: always pass catalogue radii (incl. cercle), never null-for-cercle.
  const shape0 = opts.shape ? (SHAPE_BY_ID.get(opts.shape)?.radii ?? null) : null;
  let restExpressionId = expressionId;
  let cycle: MontageBlock[] = opts.cycle?.length ? opts.cycle.slice() : [];
  let playing = opts.playing ?? cycle.length > 0;
  let blockIndex = 0;
  let blockStart = 0;
  let nextAt = Infinity;
  let clock = 0;
  let last = 0;
  let onBlock: ((index: number, block: MontageBlock) => void) | null = null;
  const initialState: StateId =
    cycle.length > 0 ? cycle[0]!.state : (opts.state ?? "idle");
  const engine = new BotEngine(R, initialState, shape0, expr0);

  function applyBlock(i: number, from = 0) {
    if (!cycle.length) {
      nextAt = Infinity;
      return;
    }
    const idx = ((i % cycle.length) + cycle.length) % cycle.length;
    const b = cycle[idx]!;
    blockIndex = idx;
    blockStart = clock - from;
    engine.setState(b.state, clock);
    const exprKey = b.expression || restExpressionId;
    if (exprKey) {
      expressionId = exprKey;
      engine.setExpression(EXPRESSION_BY_ID.get(exprKey) ?? null, clock);
    }
    nextAt = playing ? blockStart + b.duration : Infinity;
    try {
      onBlock?.(idx, b);
    } catch (_) { /* ignore */ }
  }

  const svgNS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("viewBox", `${-half} ${-half} ${half * 2} ${half * 2}`);
  svg.setAttribute("width", String(opts.size));
  svg.setAttribute("height", String(opts.size));
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "Bloub");
  svg.setAttribute("overflow", "visible");
  if (opts.className) svg.setAttribute("class", opts.className);
  svg.style.display = "block";
  svg.style.flex = "0 0 auto";
  svg.style.color = ink;

  const defs = document.createElementNS(svgNS, "defs");
  svg.appendChild(defs);

  const gArcsBack = document.createElementNS(svgNS, "g");
  gArcsBack.setAttribute("fill", "none");
  gArcsBack.setAttribute("stroke-linecap", "round");
  svg.appendChild(gArcsBack);

  const gDotsBehind = document.createElementNS(svgNS, "g");
  svg.appendChild(gDotsBehind);

  const gBody = document.createElementNS(svgNS, "g");
  const bodyPath = document.createElementNS(svgNS, "path");
  bodyPath.setAttribute("class", "cz-body");
  bodyPath.setAttribute("fill", "currentColor");
  gBody.appendChild(bodyPath);

  // Clip eyes to body silhouette (upstream uses mask holes; clipPath keeps eyes inside
  // the filled shape without relying on mask-in-srcdoc, which breaks in Host shortcuts).
  const clipId = `bloub-clip-${id}`;
  const clipPath = document.createElementNS(svgNS, "clipPath");
  clipPath.setAttribute("id", clipId);
  clipPath.setAttribute("clipPathUnits", "userSpaceOnUse");
  const clipBody = document.createElementNS(svgNS, "path");
  clipPath.appendChild(clipBody);
  defs.appendChild(clipPath);

  const gEyes = document.createElementNS(svgNS, "g");
  gEyes.setAttribute("clip-path", `url(#${clipId})`);
  gBody.appendChild(gEyes);

  // Overlay eyes (simple) — always present; hidden when alpha=0
  const eyeEls: SVGPathElement[] = [];
  for (let i = 0; i < 2; i++) {
    const p = document.createElementNS(svgNS, "path");
    p.setAttribute("class", "cz-eye");
    p.setAttribute("fill", paper);
    eyeEls.push(p);
    gEyes.appendChild(p);
  }

  // Optional mask mode (popup) — kept for edge clipping when simple=false
  let maskBody: SVGPathElement | null = null;
  let maskEyes: SVGPathElement[] = [];
  let maskNotch: SVGCircleElement | null = null;
  let paperPath: SVGPathElement | null = null;
  let inkRect: SVGRectElement | null = null;

  if (!simple) {
    const mask = document.createElementNS(svgNS, "mask");
    mask.setAttribute("id", maskId);
    mask.setAttribute("maskUnits", "userSpaceOnUse");
    mask.setAttribute("x", String(-half));
    mask.setAttribute("y", String(-half));
    mask.setAttribute("width", String(half * 2));
    mask.setAttribute("height", String(half * 2));
    maskBody = document.createElementNS(svgNS, "path");
    maskBody.setAttribute("fill", "#fff");
    mask.appendChild(maskBody);
    maskEyes = [];
    for (let i = 0; i < 2; i++) {
      const p = document.createElementNS(svgNS, "path");
      p.setAttribute("fill", "#000");
      mask.appendChild(p);
      maskEyes.push(p);
    }
    maskNotch = document.createElementNS(svgNS, "circle");
    maskNotch.setAttribute("fill", "#000");
    maskNotch.style.display = "none";
    mask.appendChild(maskNotch);
    defs.appendChild(mask);

    paperPath = document.createElementNS(svgNS, "path");
    const gMasked = document.createElementNS(svgNS, "g");
    gMasked.setAttribute("mask", `url(#${maskId})`);
    inkRect = document.createElementNS(svgNS, "rect");
    inkRect.setAttribute("x", String(-half));
    inkRect.setAttribute("y", String(-half));
    inkRect.setAttribute("width", String(half * 2));
    inkRect.setAttribute("height", String(half * 2));
    inkRect.setAttribute("fill", ink);
    gMasked.appendChild(inkRect);
    // Replace simple body with mask stack
    while (gBody.firstChild) gBody.removeChild(gBody.firstChild);
    gBody.appendChild(paperPath);
    gBody.appendChild(gMasked);
  }

  svg.appendChild(gBody);

  const gDotsFront = document.createElementNS(svgNS, "g");
  svg.appendChild(gDotsFront);

  const notifEl = document.createElementNS(svgNS, "circle");
  notifEl.setAttribute("fill", NOTIF_BLUE);
  notifEl.style.display = "none";
  svg.appendChild(notifEl);

  const gArcsFront = document.createElementNS(svgNS, "g");
  gArcsFront.setAttribute("fill", "none");
  gArcsFront.setAttribute("stroke-linecap", "round");
  svg.appendChild(gArcsFront);

  host.appendChild(svg);

  const gradMap = new Map<string, SVGLinearGradientElement>();

  function ensureGrad(arcId: string, grad: BotFrame["arcs"][number]["grad"]) {
    const key = `${id}-${arcId}`;
    let el = gradMap.get(key);
    if (!el) {
      el = document.createElementNS(svgNS, "linearGradient");
      el.setAttribute("id", key);
      el.setAttribute("gradientUnits", "userSpaceOnUse");
      defs.appendChild(el);
      gradMap.set(key, el);
    }
    el.setAttribute("x1", String(grad.x1));
    el.setAttribute("y1", String(grad.y1));
    el.setAttribute("x2", String(grad.x2));
    el.setAttribute("y2", String(grad.y2));
    while (el.firstChild) el.removeChild(el.firstChild);
    const n = grad.stops.length;
    grad.stops.forEach((c, i) => {
      const stop = document.createElementNS(svgNS, "stop");
      stop.setAttribute("offset", String(n <= 1 ? 0 : i / (n - 1)));
      stop.setAttribute("stop-color", c);
      el!.appendChild(stop);
    });
    return key;
  }

  function clearGroup(g: SVGGElement) {
    while (g.firstChild) g.removeChild(g.firstChild);
  }

  function appendDot(g: SVGGElement, dot: BotFrame["dots"][number]) {
    const fill =
      dot.color ??
      (dot.depth === undefined ? ink : mixHex(paper, ink, dot.depth));
    if (dot.d) {
      const p = document.createElementNS(svgNS, "path");
      p.setAttribute("d", dot.d);
      p.setAttribute("fill", fill);
      p.setAttribute("opacity", String(dot.opacity));
      p.setAttribute(
        "transform",
        `translate(${dot.x} ${dot.y}) rotate(${dot.rot ?? 0}) scale(${R})`,
      );
      g.appendChild(p);
    } else {
      const c = document.createElementNS(svgNS, "circle");
      c.setAttribute("cx", String(dot.x));
      c.setAttribute("cy", String(dot.y));
      c.setAttribute("r", String(dot.r));
      c.setAttribute("fill", fill);
      c.setAttribute("opacity", String(dot.opacity));
      g.appendChild(c);
    }
  }

  function paint(frame: BotFrame) {
    gBody.setAttribute("opacity", String(frame.bodyAlpha));

    if (simple) {
      bodyPath.setAttribute("d", frame.bodyPath);
      bodyPath.setAttribute("fill", "currentColor");
      clipBody.setAttribute("d", frame.bodyPath);
      frame.eyes.forEach((eye, i) => {
        const el = eyeEls[i];
        if (!el) return;
        if (!eye.d || eye.alpha < 0.02) {
          el.style.display = "none";
          return;
        }
        el.style.display = "";
        el.setAttribute("d", eye.d);
        el.setAttribute("transform", eye.matrix);
        el.setAttribute("opacity", String(eye.alpha));
        el.setAttribute("fill", paper);
      });
    } else if (maskBody && paperPath && inkRect) {
      maskBody.setAttribute("d", frame.bodyPath);
      frame.eyes.forEach((eye, i) => {
        const el = maskEyes[i];
        if (!el) return;
        el.setAttribute("d", eye.d);
        el.setAttribute("transform", eye.matrix);
        el.setAttribute("opacity", String(eye.alpha));
      });
      if (frame.notch && maskNotch) {
        maskNotch.style.display = "";
        maskNotch.setAttribute("cx", String(frame.notch.x));
        maskNotch.setAttribute("cy", String(frame.notch.y));
        maskNotch.setAttribute("r", String(frame.notch.r));
      } else if (maskNotch) {
        maskNotch.style.display = "none";
      }
      paperPath.setAttribute("d", frame.bodyPath);
      paperPath.setAttribute("fill", paper);
      inkRect.setAttribute("fill", ink);
    }

    clearGroup(gArcsBack);
    clearGroup(gArcsFront);
    for (const arc of frame.arcs) {
      const gradId = ensureGrad(arc.id, arc.grad);
      const back = document.createElementNS(svgNS, "path");
      back.setAttribute("d", arc.back);
      back.setAttribute("stroke", `url(#${gradId})`);
      back.setAttribute("stroke-width", String(arc.width));
      back.setAttribute("opacity", String(arc.opacity));
      gArcsBack.appendChild(back);
      const front = document.createElementNS(svgNS, "path");
      front.setAttribute("d", arc.front);
      front.setAttribute("stroke", `url(#${gradId})`);
      front.setAttribute("stroke-width", String(arc.width));
      front.setAttribute("opacity", String(arc.opacity));
      gArcsFront.appendChild(front);
    }

    clearGroup(gDotsBehind);
    clearGroup(gDotsFront);
    const dotHost = frame.dotsBehind ? gDotsBehind : gDotsFront;
    for (const dot of frame.dots) appendDot(dotHost, dot);

    if (frame.notif) {
      notifEl.style.display = "";
      notifEl.setAttribute("cx", String(frame.notif.x));
      notifEl.setAttribute("cy", String(frame.notif.y));
      notifEl.setAttribute("r", String(frame.notif.r));
    } else {
      notifEl.style.display = "none";
    }
  }

  let raf = 0;
  let pointer: { x: number; y: number } | null = null;
  let aiming = false;
  let turnSince = 0;
  let followOn = !!opts.follow;
  const LOOK_MORPH = BotEngine.LOOK_MORPH;

  function aimLook(now: number) {
    if (!followOn || !pointer) {
      if (aiming) {
        engine.setLook(null, now);
        aiming = false;
      }
      return;
    }
    const rect = svg.getBoundingClientRect();
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;
    const dx = pointer.x - cx;
    const dy = pointer.y - cy;
    const yaw = Math.atan2(dx, 120) * (180 / Math.PI);
    const pitch = Math.atan2(-dy, 120) * (180 / Math.PI);
    engine.setLook(
      {
        yaw: Math.max(-40, Math.min(40, yaw)),
        pitch: Math.max(-30, Math.min(30, pitch)),
        mix: 1,
        spin: 0,
        wander: 0,
      },
      now,
    );
    aiming = true;
    turnSince = now;
  }

  function onPointerMove(ev: PointerEvent) {
    pointer = { x: ev.clientX, y: ev.clientY };
  }
  function onPointerLeave() {
    pointer = null;
  }

  function attachFollow() {
    window.addEventListener("pointermove", onPointerMove);
    document.addEventListener("pointerleave", onPointerLeave);
  }
  function detachFollow() {
    window.removeEventListener("pointermove", onPointerMove);
    document.removeEventListener("pointerleave", onPointerLeave);
    pointer = null;
  }

  if (followOn) attachFollow();

  function tick(ts: number) {
    raf = requestAnimationFrame(tick);
    const dt = last ? Math.min((ts - last) / 1000, 0.064) : 0;
    last = ts;
    clock += dt;

    if (playing && cycle.length && clock >= nextAt) {
      applyBlock((blockIndex + 1) % cycle.length);
    }

    if (followOn) {
      aimLook(clock);
      if (!pointer && aiming && clock - turnSince > LOOK_MORPH) aiming = false;
    }
    paint(engine.sample(clock));
  }

  if (cycle.length) applyBlock(0);
  paint(engine.sample(0));
  raf = requestAnimationFrame(tick);

  return {
    svg,
    engine,
    setSize(px: number) {
      svg.setAttribute("width", String(px));
      svg.setAttribute("height", String(px));
    },
    setColors(next: BloubColors) {
      ink = next.ink;
      paper = next.paper;
      svg.style.color = ink;
      for (const el of eyeEls) el.setAttribute("fill", paper);
    },
    setExpression(exprId: string) {
      expressionId = exprId;
      restExpressionId = exprId;
      engine.setExpression(EXPRESSION_BY_ID.get(exprId) ?? null, clock);
    },
    setRestExpression(exprId: string) {
      restExpressionId = exprId;
      // Apply immediately if current block has no override
      const cur = cycle[blockIndex];
      if (!cur?.expression) {
        expressionId = exprId;
        engine.setExpression(EXPRESSION_BY_ID.get(exprId) ?? null, clock);
      }
    },
    setShape(shapeId: string | null | undefined) {
      // Same contract as upstream BloubBot `watch(shapeRadii)`.
      engine.setShape(shapeId ? (SHAPE_BY_ID.get(shapeId)?.radii ?? null) : null, clock);
    },
    setFollow(on: boolean) {
      if (on === followOn) return;
      followOn = on;
      if (on) {
        attachFollow();
      } else {
        detachFollow();
        aiming = false;
        // Release pointer look — face returns to expression/shape center pose.
        engine.setLook(null, clock);
      }
    },
    setState(state: StateId) {
      playing = false;
      nextAt = Infinity;
      engine.setState(state, clock);
    },
    setCycle(blocks: MontageBlock[] | null | undefined, restart = true) {
      cycle = blocks?.length ? blocks.slice() : [];
      playing = cycle.length > 0;
      if (!cycle.length) {
        nextAt = Infinity;
        return;
      }
      if (restart) applyBlock(0);
      else applyBlock(blockIndex % cycle.length);
    },
    setPlaying(on: boolean) {
      playing = on && cycle.length > 0;
      if (playing) {
        nextAt = blockStart + (cycle[blockIndex]?.duration ?? 2);
        if (clock >= nextAt) applyBlock((blockIndex + 1) % cycle.length);
      } else {
        nextAt = Infinity;
      }
    },
    seekBlock(index: number) {
      if (!cycle.length) return;
      applyBlock(index);
    },
    onBlockChange(cb: ((index: number, block: MontageBlock) => void) | null) {
      onBlock = cb;
    },
    getExpression() {
      return expressionId;
    },
    getBlockIndex() {
      return blockIndex;
    },
    getCycle() {
      return cycle.slice();
    },
    destroy() {
      cancelAnimationFrame(raf);
      detachFollow();
      svg.remove();
    },
  };
}

export type BloubMount = ReturnType<typeof createBloubMount>;
