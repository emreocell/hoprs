// A small stable-fluids solver: splat, divergence, Jacobi pressure, gradient subtract, advect.
// Velocity is in sim texels per second; dye lives in the red channel.
import { ADVECT, DIVERGENCE, GRADIENT, PRESSURE, SCALE, SPLAT, VERT } from "./shaders";

type GL = WebGL2RenderingContext;
type Uniforms = Record<string, WebGLUniformLocation | null>;
export type Program = { p: WebGLProgram; u: Uniforms };
type Target = { tex: WebGLTexture; fb: WebGLFramebuffer };
type Pair = { read: Target; write: Target; swap(): void };

const PRESSURE_ITERATIONS = 20;
const VELOCITY_FADE = 2.5;
const DYE_FADE = 1.6;

export function program(gl: GL, frag: string): Program {
  const p = gl.createProgram()!;
  const shaders = ([
    [gl.VERTEX_SHADER, VERT],
    [gl.FRAGMENT_SHADER, frag],
  ] as const).map(([type, src]) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    gl.attachShader(p, s);
    return s;
  });
  gl.linkProgram(p);
  // The linked program keeps the code; the shader objects are not needed any more.
  for (const s of shaders) {
    gl.detachShader(p, s);
    gl.deleteShader(s);
  }
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link failed");
  const u: Uniforms = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS) as number;
  for (let i = 0; i < n; i++) {
    const name = gl.getActiveUniform(p, i)!.name.replace(/\[0\]$/, "");
    u[name] = gl.getUniformLocation(p, name);
  }
  return { p, u };
}

function target(gl: GL, w: number, h: number): Target {
  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  const fb = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  gl.clearColor(0, 0, 0, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  return { tex, fb };
}

function pair(gl: GL, w: number, h: number): Pair {
  const ab = { read: target(gl, w, h), write: target(gl, w, h) };
  return Object.assign(ab, {
    swap() {
      [ab.read, ab.write] = [ab.write, ab.read];
    },
  });
}

export type Fluid = {
  w: number;
  h: number;
  dye(): WebGLTexture;
  /** Adds velocity (texels/s) and dye at a point in sim uv (y up); radius in sim texels. */
  splat(x: number, y: number, vx: number, vy: number, dye: number, radius: number): void;
  step(dt: number): void;
  dispose(): void;
};

export type FluidPrograms = Record<"splat" | "advect" | "divergence" | "scale" | "pressure" | "gradient", Program>;

/** Compiles the solver's passes; once per context, shared by every grid size. */
export function fluidPrograms(gl: GL): FluidPrograms {
  return {
    splat: program(gl, SPLAT),
    advect: program(gl, ADVECT),
    divergence: program(gl, DIVERGENCE),
    scale: program(gl, SCALE),
    pressure: program(gl, PRESSURE),
    gradient: program(gl, GRADIENT),
  };
}

/** The solver's render targets for one grid size. dispose() frees the targets, not the shared programs. */
export function createFluid(gl: GL, progs: FluidPrograms, w: number, h: number): Fluid {
  const velocity = pair(gl, w, h);
  const dye = pair(gl, w, h);
  const pressure = pair(gl, w, h);
  const divergence = target(gl, w, h);
  const vao = gl.createVertexArray();

  const run = (pr: Program, out: Target, set: (u: Uniforms) => void, textures: WebGLTexture[]) => {
    gl.useProgram(pr.p);
    textures.forEach((t, i) => {
      gl.activeTexture(gl.TEXTURE0 + i);
      gl.bindTexture(gl.TEXTURE_2D, t);
    });
    gl.uniform2f(pr.u.uTexel ?? null, 1 / w, 1 / h);
    set(pr.u);
    gl.bindFramebuffer(gl.FRAMEBUFFER, out.fb);
    gl.viewport(0, 0, w, h);
    gl.bindVertexArray(vao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };

  const splatInto = (p: Pair, x: number, y: number, value: [number, number, number], radius: number) => {
    run(
      progs.splat,
      p.write,
      (u) => {
        gl.uniform1i(u.uTarget, 0);
        gl.uniform1f(u.uAspect, w / h);
        gl.uniform1f(u.uRadius, (radius / h) * (radius / h));
        gl.uniform2f(u.uPoint, x, y);
        gl.uniform3f(u.uValue, ...value);
      },
      [p.read.tex],
    );
    p.swap();
  };

  return {
    w,
    h,
    dye: () => dye.read.tex,
    splat(x, y, vx, vy, amount, radius) {
      splatInto(velocity, x, y, [vx, vy, 0], radius);
      if (amount > 0) splatInto(dye, x, y, [amount, 0, 0], radius);
    },
    step(dt) {
      run(progs.divergence, divergence, (u) => gl.uniform1i(u.uVelocity, 0), [velocity.read.tex]);
      run(
        progs.scale,
        pressure.write,
        (u) => {
          gl.uniform1i(u.uSource, 0);
          gl.uniform1f(u.uValue, 0.8);
        },
        [pressure.read.tex],
      );
      pressure.swap();
      for (let i = 0; i < PRESSURE_ITERATIONS; i++) {
        run(
          progs.pressure,
          pressure.write,
          (u) => {
            gl.uniform1i(u.uPressure, 0);
            gl.uniform1i(u.uDivergence, 1);
          },
          [pressure.read.tex, divergence.tex],
        );
        pressure.swap();
      }
      run(
        progs.gradient,
        velocity.write,
        (u) => {
          gl.uniform1i(u.uPressure, 0);
          gl.uniform1i(u.uVelocity, 1);
        },
        [pressure.read.tex, velocity.read.tex],
      );
      velocity.swap();
      const advect = (p: Pair, fade: number) => {
        run(
          progs.advect,
          p.write,
          (u) => {
            gl.uniform1i(u.uVelocity, 0);
            gl.uniform1i(u.uSource, 1);
            gl.uniform1f(u.uDt, dt);
            gl.uniform1f(u.uFade, fade);
          },
          [velocity.read.tex, p.read.tex],
        );
        p.swap();
      };
      advect(velocity, VELOCITY_FADE);
      advect(dye, DYE_FADE);
    },
    dispose() {
      for (const t of [velocity.read, velocity.write, dye.read, dye.write, pressure.read, pressure.write, divergence]) {
        gl.deleteTexture(t.tex);
        gl.deleteFramebuffer(t.fb);
      }
      gl.deleteVertexArray(vao);
    },
  };
}
