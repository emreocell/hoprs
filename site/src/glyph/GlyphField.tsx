import { useEffect, useRef, useState } from "react";
import { FIELD_COLS, FIELD_SEED, staticField } from "./field";

// Enough rows to fill the hero at rest; the overflow is clipped. Same text on server and client.
const STATIC_ROWS = 80;
const STATIC_TEXT = staticField(FIELD_COLS, STATIC_ROWS, FIELD_SEED).join("\n");

/** Whether the animated field may run: motion allowed, WebGL2 with a float colour buffer, and IntersectionObserver. */
function canAnimate(): boolean {
  if (typeof window.matchMedia !== "function" || typeof window.IntersectionObserver !== "function") return false;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
  const gl = document.createElement("canvas").getContext("webgl2");
  if (!gl) return false;
  const ok = !!(gl.getExtension("EXT_color_buffer_float") || gl.getExtension("EXT_color_buffer_half_float"));
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return ok;
}

/** The hero's glyph field: always the static <pre>; a WebGL2 canvas over it only when allowed. */
export function GlyphField() {
  const [live, setLive] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- the gate reads the browser, so it runs after hydration
    if (canAnimate()) setLive(true);
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!live || !canvas) return;
    let stop: (() => void) | null = null;
    let done = false;
    const lost = () => {
      done = true;
      stop?.();
      stop = null;
      setLive(false);
    };
    canvas.addEventListener("webglcontextlost", lost);
    import("./run").then(({ startField }) => {
      if (!done) stop = startField(canvas, { cols: FIELD_COLS, onLost: lost });
    });
    return () => {
      done = true;
      canvas.removeEventListener("webglcontextlost", lost);
      stop?.();
    };
  }, [live]);

  return (
    <>
      <pre className="glyph-pre" aria-hidden="true">
        {STATIC_TEXT}
      </pre>
      {live && <canvas ref={canvasRef} className="glyph-canvas" aria-hidden="true" />}
    </>
  );
}
