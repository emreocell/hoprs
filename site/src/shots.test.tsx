import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import Connection from "./sections/Connection";
import Hero from "./sections/Hero";

afterEach(cleanup);

const publicFile = (url: string) => resolve(process.cwd(), "public", url.replace(/^\//, ""));

describe.each([
  { name: "main", slot: "shot-main", view: () => render(<Hero />), lazy: false },
  { name: "split", slot: "shot-split", view: () => render(<Connection />), lazy: true },
])("$name screenshot", ({ name, slot, view, lazy }) => {
  it("is a picture with AVIF and WebP sources and a sized PNG fallback", () => {
    const { container } = view();
    const figure = container.querySelector(`[data-slot="${slot}"]`)!;
    const sources = [...figure.querySelectorAll("picture source")];
    expect(sources.map((s) => s.getAttribute("type"))).toEqual(["image/avif", "image/webp"]);
    for (const s of sources) {
      expect(s.getAttribute("sizes")).toBeTruthy();
      for (const candidate of s.getAttribute("srcset")!.split(",")) {
        const url = candidate.trim().split(/\s+/)[0];
        expect(url).toContain(`/shots/${name}`);
        expect(existsSync(publicFile(url)), url).toBe(true);
      }
    }
    const img = figure.querySelector("picture img")!;
    expect(img.getAttribute("src")).toBe(`/shots/${name}.png`);
    expect(existsSync(publicFile(img.getAttribute("src")!))).toBe(true);
    expect(img.getAttribute("width")).toBe("1440");
    expect(img.getAttribute("height")).toBe("900");
    expect(img.getAttribute("alt")!.length).toBeGreaterThan(20);
    expect(img.getAttribute("loading")).toBe(lazy ? "lazy" : null);
    expect(img.getAttribute("fetchpriority")).toBeNull();
  });
});

it("the OG image exists", () => {
  expect(existsSync(publicFile("/og.png"))).toBe(true);
});
