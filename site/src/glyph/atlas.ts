/** Draws each character once, white on transparent, in a row of `cell`-sized slots (device pixels). */
export function buildAtlas(chars: string, cell: { w: number; h: number }, font: string): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, chars.length * cell.w);
  canvas.height = Math.max(1, cell.h);
  const ctx = canvas.getContext("2d");
  if (!ctx) return canvas;
  ctx.font = font;
  ctx.fillStyle = "#fff";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (let i = 0; i < chars.length; i++) ctx.fillText(chars[i], i * cell.w + cell.w / 2, cell.h / 2);
  return canvas;
}
