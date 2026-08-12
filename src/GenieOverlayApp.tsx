import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./genieOverlay.css";

type GenieRect = { x: number; y: number; w: number; h: number };

type GeniePlayPayload = {
  /** Absolute file path — preferred (convertFileSrc). */
  framePath?: string;
  /** Legacy JPEG base64 fallback (no data: prefix). */
  jpegBase64?: string;
  /** Rects relative to this overlay window (logical px). */
  from: GenieRect;
  to: GenieRect;
  direction: "suck" | "expand";
  durationMs?: number;
  requestId: string;
};

/** Dense shared-vertex grid — WebGL rasterizes continuous edges (no canvas clip hairlines). */
const COLS = 48;
const ROWS = 32;

function easeInOutCubic(t: number) {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

function preferReducedMotion() {
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

const VERT_SRC = `#version 300 es
precision highp float;
in vec2 a_uv;
uniform vec2 u_res;
uniform vec4 u_from;
uniform vec4 u_to;
uniform float u_progress;
out vec2 v_uv;

float easeInOutCubic(float t) {
  return t < 0.5 ? 4.0 * t * t * t : 1.0 - pow(-2.0 * t + 2.0, 3.0) / 2.0;
}

vec2 genieMap(vec2 uv, float progress, vec4 from, vec4 to) {
  float u = uv.x;
  float v = uv.y;
  // Exact window pose while progress is still in the hold zone — no premature stretch.
  if (progress <= 0.001) {
    return vec2(from.x + u * from.z, from.y + v * from.w);
  }
  // Mild bottom-lead — keep motion coherent so the top doesn't "lid slam".
  float lag = (1.0 - v) * 0.18;
  float local = clamp((progress - lag) / max(0.42, 1.0 - lag * 0.4), 0.0, 1.0);
  float e = easeInOutCubic(local);

  float iconCx = to.x + to.z * 0.5;
  float iconCy = to.y + to.w * 0.5;
  float srcX = from.x + u * from.z;
  float srcY = from.y + v * from.w;

  float funnel = e;
  float iconRatio = to.z / max(8.0, from.z);

  // Top also narrows with progress (avoids a flat lid dropping from full width).
  float wTop = mix(1.0, 0.52, funnel);
  float wBot = mix(1.0, max(0.045, iconRatio), funnel);
  // Concave: mid closer to narrow; pow(v,<1) pulls mid down the profile.
  float along = pow(clamp(v, 0.0, 1.0), 0.55);
  float waist = 4.0 * v * (1.0 - v);
  float profile = mix(wTop, wBot, along);
  profile *= 1.0 - waist * 0.38 * funnel;
  float halfW = from.z * max(0.03, profile) * 0.5;

  float midCx = mix(from.x + from.z * 0.5, iconCx, pow(funnel, 0.9));
  float x = midCx + (u - 0.5) * 2.0 * halfW;

  // Longer mid funnel: top descends slower; bottom reaches icon sooner.
  // Remap v so mid rows occupy more vertical span.
  float vLong = clamp(v + 0.28 * v * (1.0 - v) * (1.0 - v), 0.0, 1.0);
  float yTop = mix(from.y, iconCy - to.w * 1.15 - from.w * 0.08 * (1.0 - e), pow(e, 1.55));
  float yBot = mix(from.y + from.w, iconCy + to.w * 0.4, pow(e, 0.72));
  // Keep a tall funnel length (from = xywh → height is .w, not .h).
  yTop = min(yTop, mix(from.y, yBot - max(48.0, from.w * 0.22), funnel));
  float yBody = mix(yTop, yBot, vLong);
  float yDirect = mix(srcY, iconCy, e);
  float y = mix(yBody, yDirect, e * e * 0.65);

  float xBlended = mix(srcX, x, min(1.0, e * 1.05));
  return vec2(xBlended, y);
}

void main() {
  v_uv = a_uv;
  vec2 p = genieMap(a_uv, u_progress, u_from, u_to);
  // CSS px → clip space (y down in CSS, y up in clip)
  vec2 clip = vec2((p.x / u_res.x) * 2.0 - 1.0, 1.0 - (p.y / u_res.y) * 2.0);
  gl_Position = vec4(clip, 0.0, 1.0);
}
`;

const FRAG_SRC = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
out vec4 outColor;
void main() {
  vec4 c = texture(u_tex, v_uv);
  // Drop fully empty texels so cropped shadow/black margins stay invisible.
  if (c.a < 0.04 && c.r + c.g + c.b < 0.04) discard;
  outColor = c;
}
`;

type GlKit = {
  gl: WebGL2RenderingContext;
  prog: WebGLProgram;
  vao: WebGLVertexArrayObject;
  tex: WebGLTexture;
  uRes: WebGLUniformLocation;
  uFrom: WebGLUniformLocation;
  uTo: WebGLUniformLocation;
  uProgress: WebGLUniformLocation;
  indexCount: number;
};

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const sh = gl.createShader(type);
  if (!sh) throw new Error("createShader");
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(sh) || "compile fail";
    gl.deleteShader(sh);
    throw new Error(log);
  }
  return sh;
}

function buildGl(canvas: HTMLCanvasElement): GlKit | null {
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    premultipliedAlpha: true,
    antialias: true,
    preserveDrawingBuffer: true,
  });
  if (!gl) return null;

  try {
    const vs = compile(gl, gl.VERTEX_SHADER, VERT_SRC);
    const fs = compile(gl, gl.FRAGMENT_SHADER, FRAG_SRC);
    const prog = gl.createProgram();
    if (!prog) return null;
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      throw new Error(gl.getProgramInfoLog(prog) || "link fail");
    }

    // Shared-vertex UV grid (COLS×ROWS cells → (COLS+1)×(ROWS+1) verts).
    const nx = COLS + 1;
    const ny = ROWS + 1;
    const uvs = new Float32Array(nx * ny * 2);
    for (let j = 0; j < ny; j++) {
      for (let i = 0; i < nx; i++) {
        const o = (j * nx + i) * 2;
        uvs[o] = i / COLS;
        uvs[o + 1] = j / ROWS;
      }
    }
    const indices = new Uint32Array(COLS * ROWS * 6);
    let k = 0;
    for (let j = 0; j < ROWS; j++) {
      for (let i = 0; i < COLS; i++) {
        const a = j * nx + i;
        const b = a + 1;
        const c = a + nx;
        const d = c + 1;
        indices[k++] = a;
        indices[k++] = b;
        indices[k++] = d;
        indices[k++] = a;
        indices[k++] = d;
        indices[k++] = c;
      }
    }

    const vao = gl.createVertexArray();
    const vbo = gl.createBuffer();
    const ibo = gl.createBuffer();
    if (!vao || !vbo || !ibo) return null;

    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, uvs, gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, "a_uv");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
    gl.bindVertexArray(null);

    const tex = gl.createTexture();
    if (!tex) return null;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

    const uRes = gl.getUniformLocation(prog, "u_res");
    const uFrom = gl.getUniformLocation(prog, "u_from");
    const uTo = gl.getUniformLocation(prog, "u_to");
    const uProgress = gl.getUniformLocation(prog, "u_progress");
    const uTex = gl.getUniformLocation(prog, "u_tex");
    if (!uRes || !uFrom || !uTo || !uProgress || !uTex) return null;

    gl.useProgram(prog);
    gl.uniform1i(uTex, 0);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.clearColor(0, 0, 0, 0);
    // Flip so texture (u,0) = image top — matches genie UV v=0 top.
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 1);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 1);

    return {
      gl,
      prog,
      vao,
      tex,
      uRes,
      uFrom,
      uTo,
      uProgress,
      indexCount: indices.length,
    };
  } catch (e) {
    console.error("[GenieOverlay] WebGL init failed", e);
    return null;
  }
}

function uploadTexture(kit: GlKit, img: TexImageSource) {
  const { gl, tex } = kit;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
}

function drawGlFrame(
  kit: GlKit,
  cssW: number,
  cssH: number,
  from: GenieRect,
  to: GenieRect,
  progress: number,
) {
  const { gl, prog, vao, tex, uRes, uFrom, uTo, uProgress, indexCount } = kit;
  const rw = Math.max(1, cssW);
  const rh = Math.max(1, cssH);
  gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.useProgram(prog);
  gl.uniform2f(uRes, rw, rh);
  gl.uniform4f(uFrom, from.x, from.y, from.w, from.h);
  gl.uniform4f(uTo, to.x, to.y, to.w, to.h);
  gl.uniform1f(uProgress, progress);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.bindVertexArray(vao);
  gl.drawElements(gl.TRIANGLES, indexCount, gl.UNSIGNED_INT, 0);
  gl.bindVertexArray(null);
}

async function playGenie(
  kit: GlKit,
  canvas: HTMLCanvasElement,
  from: GenieRect,
  to: GenieRect,
  direction: "suck" | "expand",
  durationMs: number,
) {
  const t0 = performance.now();
  // Hold identity for the first slice so the freeze matches the live window
  // before any funnel stretch — avoids the "突然拉高" jump.
  const HOLD = preferReducedMotion() ? 0 : 0.14;
  await new Promise<void>((resolve) => {
    const tick = (now: number) => {
      const raw = Math.min(1, (now - t0) / durationMs);
      const morph = raw <= HOLD ? 0 : (raw - HOLD) / Math.max(0.001, 1 - HOLD);
      const eased = preferReducedMotion() ? morph : easeInOutCubic(morph);
      const progress = direction === "suck" ? eased : 1 - eased;

      const cssW = canvas.clientWidth || window.innerWidth;
      const cssH = canvas.clientHeight || window.innerHeight;

      if (progress < 0.008) {
        drawGlFrame(kit, cssW, cssH, from, to, 0);
      } else if (progress > 0.992) {
        if (direction === "suck") {
          drawGlFrame(kit, cssW, cssH, from, to, 1);
        } else {
          drawGlFrame(kit, cssW, cssH, from, to, 0);
        }
      } else {
        drawGlFrame(kit, cssW, cssH, from, to, progress);
      }

      if (raw < 1) requestAnimationFrame(tick);
      else resolve();
    };
    requestAnimationFrame(tick);
  });
}

export default function GenieOverlayApp() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const busyRef = useRef(false);
  const kitRef = useRef<GlKit | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const fitCanvas = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = window.innerWidth;
      const h = window.innerHeight;
      const bw = Math.max(1, Math.round(w * dpr));
      const bh = Math.max(1, Math.round(h * dpr));
      if (canvas.width !== bw || canvas.height !== bh) {
        canvas.width = bw;
        canvas.height = bh;
      }
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      const kit = kitRef.current;
      if (kit) {
        kit.gl.viewport(0, 0, bw, bh);
      }
    };

    // WebGL context must be created once; switching 2d/webgl is impossible on same canvas.
    kitRef.current = buildGl(canvas);
    fitCanvas();
    window.addEventListener("resize", fitCanvas);

    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void listen<GeniePlayPayload>("genie-play", (ev) => {
      const p = ev.payload;
      console.info("[GenieOverlay] play", p?.direction, p?.requestId, p?.framePath || "b64");
      if (cancelled || busyRef.current) {
        console.warn("[GenieOverlay] busy/cancelled, drop play");
        if (p?.requestId) {
          void invoke("genie_overlay_done", { requestId: p.requestId }).catch(() => undefined);
        }
        return;
      }
      if ((!p?.framePath && !p?.jpegBase64) || !p.requestId) return;
      const kit = kitRef.current;
      if (!kit) {
        console.error("[GenieOverlay] WebGL unavailable");
        void invoke("genie_overlay_done", { requestId: p.requestId }).catch(() => undefined);
        return;
      }

      busyRef.current = true;
      void (async () => {
        try {
          fitCanvas();
          let blob: Blob;
          if (p.framePath) {
            const { convertFileSrc } = await import("@tauri-apps/api/core");
            const url = convertFileSrc(p.framePath);
            blob = await fetch(url).then((r) => {
              if (!r.ok) throw new Error(`frame fetch ${r.status}`);
              return r.blob();
            });
          } else {
            blob = await fetch(`data:image/jpeg;base64,${p.jpegBase64}`).then((r) => r.blob());
          }
          const bitmap = await createImageBitmap(blob);
          uploadTexture(kit, bitmap);

          const cssW = canvas.clientWidth || window.innerWidth;
          const cssH = canvas.clientHeight || window.innerHeight;

          if (p.direction === "expand") {
            drawGlFrame(kit, cssW, cssH, p.from, p.to, 1);
          } else {
            drawGlFrame(kit, cssW, cssH, p.from, p.to, 0);
          }
          kit.gl.flush();

          void invoke("genie_overlay_painted", { requestId: p.requestId }).catch(() => undefined);

          const duration = preferReducedMotion()
            ? 120
            : Math.max(200, Math.min(1500, p.durationMs ?? 560));
          await playGenie(kit, canvas, p.from, p.to, p.direction, duration);

          if (p.direction === "expand") {
            drawGlFrame(kit, cssW, cssH, p.from, p.to, 0);
          }

          try {
            await invoke("genie_overlay_done", { requestId: p.requestId });
          } catch {
            /* noop */
          }
          bitmap.close();
        } catch (e) {
          console.error("[GenieOverlay]", e);
          try {
            await invoke("genie_overlay_done", { requestId: p.requestId });
          } catch {
            /* noop */
          }
        } finally {
          busyRef.current = false;
        }
      })();
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    return () => {
      cancelled = true;
      window.removeEventListener("resize", fitCanvas);
      unsubs.forEach((fn) => fn());
      const kit = kitRef.current;
      if (kit) {
        const { gl, prog, vao, tex } = kit;
        gl.deleteTexture(tex);
        gl.deleteVertexArray(vao);
        gl.deleteProgram(prog);
        kitRef.current = null;
      }
    };
  }, []);

  return (
    <div className="genie-overlay-root" aria-hidden>
      <canvas ref={canvasRef} className="genie-overlay-canvas" />
    </div>
  );
}
