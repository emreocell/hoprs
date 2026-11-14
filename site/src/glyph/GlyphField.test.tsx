import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

// The WebGL renderer is its own chunk; here it is a stand-in loop that runs on rAF until stopped.
vi.mock("./run", () => ({
  startField: vi.fn(() => {
    let id = 0;
    const tick = () => {
      id = requestAnimationFrame(tick);
    };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }),
}));

import { GlyphField } from "./GlyphField";

let reduced = false;
let gl: object | null = null;
const raf = vi.fn((cb: FrameRequestCallback) => setTimeout(() => cb(performance.now()), 16) as unknown as number);
const caf = vi.fn((id: number) => clearTimeout(id));

class FakeIO {
  observe() {}
  disconnect() {}
  unobserve() {}
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  reduced = false;
  gl = { getExtension: () => ({ loseContext: () => {} }) };
  raf.mockClear();
  vi.stubGlobal("requestAnimationFrame", raf);
  vi.stubGlobal("cancelAnimationFrame", caf);
  vi.stubGlobal("IntersectionObserver", FakeIO);
  vi.stubGlobal(
    "matchMedia",
    vi.fn((q: string) => ({ matches: q.includes("reduce") ? reduced : false, media: q, addEventListener() {}, removeEventListener() {} })),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => gl) as never);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const pre = (c: HTMLElement) => c.querySelector('pre[aria-hidden="true"]') as HTMLElement | null;

it("reduced motion never starts the loop", async () => {
  reduced = true;
  const { container } = render(<GlyphField />);
  await act(() => vi.advanceTimersByTimeAsync(100));
  expect(container.querySelector("canvas")).toBeNull();
  expect(pre(container)).not.toBeNull();
  expect(raf).not.toHaveBeenCalled();
});

it("no WebGL2 keeps the static field", async () => {
  gl = null;
  const { container } = render(<GlyphField />);
  await act(() => vi.advanceTimersByTimeAsync(100));
  expect(container.querySelector("canvas")).toBeNull();
  expect(pre(container)).not.toBeNull();
  expect(raf).not.toHaveBeenCalled();
});

it("context loss shows the static field", async () => {
  const { container } = render(<GlyphField />);
  await waitFor(() => expect(container.querySelector("canvas")).not.toBeNull());
  await waitFor(() => expect(raf).toHaveBeenCalled());
  act(() => {
    container.querySelector("canvas")!.dispatchEvent(new Event("webglcontextlost"));
  });
  expect(container.querySelector("canvas")).toBeNull();
  expect(pre(container)).not.toBeNull();
  expect(pre(container)!.hidden).toBe(false);
  const calls = raf.mock.calls.length;
  await act(() => vi.advanceTimersByTimeAsync(200));
  expect(raf.mock.calls.length).toBe(calls);
});
