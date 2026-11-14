import { glyphAt } from "./vocabulary";

/** Grid of the hero field: 130 columns, cells 1.85 times as tall as wide. */
export const FIELD_COLS = 130;
export const CELL_ASPECT = 1.85;
export const FIELD_SEED = 7;

/** Density to a level 0..8 (0 is blank). The display shader quantises the same way. */
export function levelFor(density: number): number {
  const d = Math.min(1, Math.max(0, density));
  return Math.floor(d * 8 + 0.5);
}

function hash(x: number, y: number, seed: number): number {
  let h = Math.imul(x, 374761393) ^ Math.imul(y, 668265263) ^ Math.imul(seed, 2147483647);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

function smooth(t: number): number {
  return t * t * (3 - 2 * t);
}

/** Value noise in [0,1). Depends only on position and seed, so a taller field keeps the same top rows. */
function noise(x: number, y: number, seed: number): number {
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const u = smooth(x - xi);
  const v = smooth(y - yi);
  const a = hash(xi, yi, seed);
  const b = hash(xi + 1, yi, seed);
  const c = hash(xi, yi + 1, seed);
  const d = hash(xi + 1, yi + 1, seed);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}

/** Whether a cell is lit in the field at rest: faint wisps of dye. */
export function restLit(col: number, row: number, seed: number): boolean {
  const x = col;
  const y = row * CELL_ASPECT;
  const n = 0.6 * noise(x / 22, y / 22, seed) + 0.3 * noise(x / 9, y / 9, seed + 1) + 0.1 * hash(col, row, seed + 2);
  return n > 0.62;
}

/** The field at rest as text rows, deterministic for (cols, rows, seed). Cell i shows glyphAt(i). */
export function staticField(cols: number, rows: number, seed: number): string[] {
  const out: string[] = [];
  for (let r = 0; r < rows; r++) {
    let line = "";
    for (let c = 0; c < cols; c++) line += restLit(c, r, seed) ? glyphAt(r * cols + c) : " ";
    out.push(line);
  }
  return out;
}

/**
 * Where an ambient stroke starts across the canvas (0..1): right of the text column (45% to 90% of what is
 * on screen), scaled to the visible fraction of the canvas so narrow screens, which clip half of it, see it.
 */
export function ambientSpawnX(rand: number, visible: number): number {
  return visible * (0.45 + Math.min(1, Math.max(0, rand)) * 0.45);
}
