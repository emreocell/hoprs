import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startField } from "./run";

// A WebGL2 context that records calls: GL constants are numbers, methods are spies returning fresh handles.
function fakeGL() {
  const loseContext = vi.fn();
  const overrides: Record<string, unknown> = {
    getExtension: vi.fn((name: string) => (name === "WEBGL_lose_context" ? { loseContext } : {})),
    getProgramParameter: vi.fn((_p: unknown, pname: number) => (pname === 0x8b82 ? true : 0)),
    isContextLost: vi.fn(() => false),
    LINK_STATUS: 0x8b82,
  };
  const spies: Record<string, ReturnType<typeof vi.fn>> = {};
  const gl = new Proxy(
    {},
    {
      get(_t, prop: string) {
        if (prop in overrides) return overrides[prop];
        if (/^[A-Z0-9_]+$/.test(prop)) return 1;
        if (prop === "then") return undefined;
        return (spies[prop] ??= vi.fn(() => ({})));
      },
    },
  );
  const calls = (name: string) => spies[name]?.mock.calls.length ?? 0;
  return { gl, loseContext, calls };
}

let frames: FrameRequestCallback[] = [];
const flushFrame = () => {
  const due = frames;
  frames = [];
  due.forEach((cb) => cb(performance.now()));
};

beforeEach(() => {
  frames = [];
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function started() {
  const fake = fakeGL();
  const canvas = document.createElement("canvas");
  document.createElement("div").append(canvas);
  vi.spyOn(canvas, "getContext").mockImplementation(((kind: string) => (kind === "webgl2" ? fake.gl : null)) as never);
  // The atlas and the ramp probe draw on 2D canvases; jsdom has none, so they come back blank.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => null) as never);
  const stop = startField(canvas, { cols: 130, onLost: () => {} });
  await new Promise((r) => setTimeout(r, 0)); // fonts.load (absent in jsdom) → programs compiled
  flushFrame(); // first frame: layout, atlas, solver targets
  return { ...fake, stop };
}

it("compiles the programs once and frees the shader objects after linking", async () => {
  const { calls, stop } = await started();
  expect(calls("createProgram")).toBe(7); // six solver passes + display
  expect(calls("deleteShader")).toBe(14);
  window.dispatchEvent(new Event("resize"));
  flushFrame();
  expect(calls("createProgram")).toBe(7);
  stop();
});

it("stop frees GPU resources and releases the context", async () => {
  const { calls, loseContext, stop } = await started();
  stop();
  expect(calls("deleteProgram")).toBe(7);
  expect(calls("deleteTexture")).toBeGreaterThanOrEqual(9); // 7 solver targets + atlas + cells
  expect(calls("deleteFramebuffer")).toBe(7);
  expect(loseContext).toHaveBeenCalledTimes(1);
  stop();
  expect(loseContext).toHaveBeenCalledTimes(1);
  const draws = calls("drawArrays");
  flushFrame(); // a frame already queued when stop ran does nothing and queues nothing
  expect(calls("drawArrays")).toBe(draws);
  expect(frames).toHaveLength(0);
});
