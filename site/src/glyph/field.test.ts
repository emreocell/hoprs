import { expect, it } from "vitest";
import { levelFor, staticField } from "./field";
import { glyphAt, VOCAB } from "./vocabulary";

it("levelFor clamps and quantises density to 0..8", () => {
  expect(levelFor(-1)).toBe(0);
  expect(levelFor(0)).toBe(0);
  expect(levelFor(0.5)).toBe(4);
  expect(levelFor(1)).toBe(8);
  expect(levelFor(2)).toBe(8);
});

it("staticField is deterministic, sized, and faint", () => {
  const a = staticField(130, 40, 7);
  expect(a).toHaveLength(40);
  for (const row of a) expect(row).toHaveLength(130);
  expect(staticField(130, 40, 7)).toEqual(a);
  expect(staticField(130, 40, 8)).not.toEqual(a);
  const lit = a.join("").replace(/ /g, "").length;
  expect(lit / (130 * 40)).toBeLessThanOrEqual(0.35);
  expect(lit).toBeGreaterThan(0);
});

it("glyphAt never returns a space", () => {
  expect(VOCAB.length).toBeGreaterThanOrEqual(200);
  expect(/^[\x20-\x7e]+$/.test(VOCAB)).toBe(true);
  for (let i = 0; i < VOCAB.length * 3; i++) expect(glyphAt(i)).not.toBe(" ");
});
