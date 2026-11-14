// The animated hero field: a fluid stirred by the pointer, shown as hoprs' vocabulary in a grey ramp.
import { buildAtlas } from "./atlas";
import { ambientSpawnX, CELL_ASPECT, FIELD_SEED, restLit } from "./field";
import { createFluid, fluidPrograms, program, type Fluid, type FluidPrograms } from "./fluid";
import { DISPLAY } from "./shaders";
import { GLYPH_SET, glyphAt } from "./vocabulary";

const FONT_FAMILY = '"JetBrains Mono Variable", ui-monospace, monospace';
const ADVANCE = 0.6; // JetBrains Mono advance width, in em
const SIM_COLS = 128;
const DPR_CAP = 2;
const AMBIENT_IDLE_MS = 4000;
const AMBIENT_EVERY_MS = 3000;
const AMBIENT_STROKE_MS = 700;

/** Reads `--glyph-0..7` as sRGB triples in 0..1, whatever colour syntax the tokens use. */
function readRamp(): Float32Array<ArrayBuffer> {
  const ramp = new Float32Array(24);
  const style = getComputedStyle(document.documentElement);
  const probe = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  for (let i = 0; i < 8; i++) {
    let rgb = [0.3 + i * 0.095, 0.3 + i * 0.095, 0.3 + i * 0.095];
    const value = style.getPropertyValue(`--glyph-${i}`).trim();
    if (probe && value) {
      probe.clearRect(0, 0, 1, 1);
      probe.fillStyle = value;
      probe.fillRect(0, 0, 1, 1);
      const d = probe.getImageData(0, 0, 1, 1).data;
      rgb = [d[0] / 255, d[1] / 255, d[2] / 255];
    }
    ramp.set(rgb, i * 3);
  }
  return ramp;
}

// visible: the fraction of the canvas width on screen (the slot clips the rest; half on narrow screens).
type Layout = { cols: number; rows: number; cellW: number; cellH: number; dpr: number; visible: number };

export function startField(canvas: HTMLCanvasElement, opts: { cols: number; onLost: () => void }): () => void {
  const { cols, onLost } = opts;
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    premultipliedAlpha: true,
    antialias: false,
    depth: false,
    stencil: false,
    powerPreference: "low-power",
  });
  if (!gl) {
    queueMicrotask(onLost);
    return () => {};
  }
  gl.getExtension("EXT_color_buffer_float");
  gl.getExtension("EXT_color_buffer_half_float");

  let stopped = false;
  let frame = 0;
  let inView = true;
  let visible = document.visibilityState !== "hidden";
  let last = 0;
  let layout: Layout | null = null;
  let fluid: Fluid | null = null;
  let solver: FluidPrograms | null = null;
  let display: ReturnType<typeof program> | null = null;
  let atlasTex: WebGLTexture | null = null;
  let cellsTex: WebGLTexture | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  let ramp: Float32Array<ArrayBuffer> = new Float32Array(24);
  let resized = true;

  const coarse = window.matchMedia("(pointer: coarse)").matches;
  let pointer: { x: number; y: number } | null = null;
  let moved = { dx: 0, dy: 0 };
  let lastPointer = -Infinity;
  let lastAmbient = performance.now() - AMBIENT_EVERY_MS + 600;
  let stroke: { t0: number; x: number; y: number; vx: number; vy: number; bend: number } | null = null;

  const measure = (): Layout => {
    const dpr = Math.min(DPR_CAP, window.devicePixelRatio || 1);
    const fontSize = parseFloat(getComputedStyle(canvas).fontSize) || 16;
    // Whole device pixels per cell keep the glyphs crisp.
    const cellW = Math.max(2, Math.ceil(fontSize * ADVANCE * dpr));
    const cellH = Math.max(3, Math.round(cellW * CELL_ASPECT));
    const height = canvas.parentElement?.clientHeight || canvas.clientHeight || 400;
    const rows = Math.max(1, Math.ceil((height * dpr) / cellH));
    canvas.width = cols * cellW;
    canvas.height = rows * cellH;
    canvas.style.width = `${(cols * cellW) / dpr}px`;
    canvas.style.height = `${(rows * cellH) / dpr}px`;
    const width = canvas.parentElement?.clientWidth || (cols * cellW) / dpr;
    const visible = Math.min(1, width / ((cols * cellW) / dpr));
    return { cols, rows, cellW, cellH, dpr, visible };
  };

  const texture = (filter: number) => {
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  };

  const rebuild = () => {
    const prev = layout;
    layout = measure();
    const { rows, cellW, cellH } = layout;
    if (!prev || prev.cellW !== cellW || prev.cellH !== cellH) {
      if (atlasTex) gl.deleteTexture(atlasTex);
      atlasTex = texture(gl.LINEAR);
      const font = `400 ${(cellW / ADVANCE).toFixed(2)}px ${FONT_FAMILY}`;
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, buildAtlas(GLYPH_SET, { w: cellW, h: cellH }, font));
    }
    if (!prev || prev.rows !== rows) {
      const data = new Uint8Array(cols * rows * 2);
      for (let r = 0; r < rows; r++)
        for (let c = 0; c < cols; c++) {
          const i = r * cols + c;
          data[i * 2] = GLYPH_SET.indexOf(glyphAt(i));
          data[i * 2 + 1] = restLit(c, r, FIELD_SEED) ? 1 : 0;
        }
      if (cellsTex) gl.deleteTexture(cellsTex);
      cellsTex = texture(gl.NEAREST);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG8UI, cols, rows, 0, gl.RG_INTEGER, gl.UNSIGNED_BYTE, data);
    }
    const simH = Math.max(16, Math.round((SIM_COLS * canvas.height) / canvas.width));
    if (!fluid || fluid.h !== simH) {
      fluid?.dispose();
      fluid = createFluid(gl, solver!, SIM_COLS, simH);
    }
  };

  const toSim = (clientX: number, clientY: number) => {
    const rect = canvas.getBoundingClientRect();
    return { x: (clientX - rect.left) / rect.width, y: 1 - (clientY - rect.top) / rect.height };
  };

  const onPointer = (e: PointerEvent) => {
    const p = toSim(e.clientX, e.clientY);
    if (pointer && p.x >= 0 && p.x <= 1 && p.y >= 0 && p.y <= 1) {
      moved.dx += p.x - pointer.x;
      moved.dy += p.y - pointer.y;
    }
    pointer = p;
    if (e.pointerType !== "touch") lastPointer = performance.now();
  };

  const draw = () => {
    if (!layout || !fluid || !display) return;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(display.p);
    const u = display.u;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, fluid.dye());
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, atlasTex);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, cellsTex);
    gl.uniform1i(u.uDye, 0);
    gl.uniform1i(u.uAtlas, 1);
    gl.uniform1i(u.uCells, 2);
    gl.uniform2f(u.uGrid, layout.cols, layout.rows);
    gl.uniform2f(u.uCanvas, canvas.width, canvas.height);
    gl.uniform1f(u.uSlots, GLYPH_SET.length);
    gl.uniform3fv(u.uRamp, ramp);
    gl.bindVertexArray(vao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };

  const tick = (now: number) => {
    frame = 0;
    if (stopped || gl.isContextLost()) return;
    if (resized) {
      resized = false;
      rebuild();
    }
    const f = fluid!;
    const dt = Math.min(1 / 30, Math.max(1 / 240, (now - (last || now - 16)) / 1000));
    last = now;

    // The pointer: velocity follows its motion, dye grows with its speed.
    if (pointer && (moved.dx !== 0 || moved.dy !== 0)) {
      const px = Math.hypot(moved.dx * canvas.width, moved.dy * canvas.height) / layout!.dpr;
      f.splat(pointer.x, pointer.y, (moved.dx * f.w * 1.6) / dt, (moved.dy * f.h * 1.6) / dt, Math.min(0.85, 0.1 + px / 24), 2.5);
      moved = { dx: 0, dy: 0 };
    }

    // Ambient strokes when the pointer is idle (always on touch screens), so the field still moves.
    if (!stroke && (coarse || now - lastPointer > AMBIENT_IDLE_MS) && now - lastAmbient > AMBIENT_EVERY_MS) {
      const a = Math.random() * Math.PI * 2;
      const speed = 0.25 + Math.random() * 0.2;
      stroke = {
        t0: now,
        x: ambientSpawnX(Math.random(), layout!.visible),
        y: 0.15 + Math.random() * 0.7,
        vx: Math.cos(a) * speed * (canvas.height / canvas.width),
        vy: Math.sin(a) * speed,
        bend: (Math.random() - 0.5) * 3,
      };
    }
    if (stroke) {
      const t = (now - stroke.t0) / 1000;
      if (t * 1000 > AMBIENT_STROKE_MS) {
        stroke = null;
        lastAmbient = now;
      } else {
        const c = Math.cos(stroke.bend * t);
        const s = Math.sin(stroke.bend * t);
        const vx = stroke.vx * c - stroke.vy * s;
        const vy = stroke.vx * s + stroke.vy * c;
        stroke.x += vx * dt;
        stroke.y += vy * dt;
        f.splat(stroke.x, stroke.y, vx * f.w * 1.6, vy * f.h * 1.6, 0.3, 2.5);
      }
    }

    f.step(dt);
    draw();
    canvas.dataset.ready = "";
    schedule();
  };

  const schedule = () => {
    if (!stopped && !frame && inView && visible && display) frame = requestAnimationFrame(tick);
  };
  const pause = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    last = 0;
  };
  const onVisibility = () => {
    visible = document.visibilityState !== "hidden";
    if (visible) schedule();
    else pause();
  };
  const onResize = () => {
    resized = true;
  };
  const io = new IntersectionObserver((entries) => {
    inView = entries.some((e) => e.isIntersecting);
    if (inView) schedule();
    else pause();
  });

  const stop = () => {
    if (stopped) return;
    stopped = true;
    pause();
    io.disconnect();
    window.removeEventListener("pointermove", onPointer);
    window.removeEventListener("resize", onResize);
    document.removeEventListener("visibilitychange", onVisibility);
    canvas.removeEventListener("webglcontextlost", onContextLost);
    // Free the GPU side and hand the context back (browsers keep only ~16 live).
    if (!gl.isContextLost()) {
      fluid?.dispose();
      for (const pr of [...Object.values(solver ?? {}), display]) if (pr) gl.deleteProgram(pr.p);
      for (const t of [atlasTex, cellsTex]) if (t) gl.deleteTexture(t);
      if (vao) gl.deleteVertexArray(vao);
      gl.getExtension("WEBGL_lose_context")?.loseContext();
    }
    fluid = solver = display = null;
    atlasTex = cellsTex = vao = null;
  };
  function onContextLost() {
    stop();
    onLost();
  }

  canvas.addEventListener("webglcontextlost", onContextLost);
  window.addEventListener("pointermove", onPointer, { passive: true });
  window.addEventListener("resize", onResize, { passive: true });
  document.addEventListener("visibilitychange", onVisibility);
  io.observe(canvas);

  // The atlas needs the real font; build once it has loaded (or failed, then the fallback mono face is used).
  const fontReady = document.fonts?.load(`16px ${FONT_FAMILY}`) ?? Promise.resolve();
  fontReady
    .catch(() => undefined)
    .then(() => {
      if (stopped) return;
      try {
        ramp = readRamp();
        solver = fluidPrograms(gl);
        display = program(gl, DISPLAY);
        vao = gl.createVertexArray();
        schedule();
      } catch {
        onContextLost();
      }
    });

  return stop;
}
