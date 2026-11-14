import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import App from "./App";
import OsLogo from "./components/OsLogo";
import { dicts, formatDate, formatNumber } from "./i18n";
import type { Dict } from "./i18n";

afterEach(cleanup);

// Every leaf of a dictionary as [path, text]; functions are called with a sample argument.
function leaves(value: unknown, path = ""): [string, string][] {
  if (typeof value === "string") return [[path, value]];
  if (typeof value === "function") return [[path, (value as (s: string) => string)("1")]];
  if (Array.isArray(value)) return value.flatMap((v, i) => leaves(v, `${path}[${i}]`));
  return Object.entries(value as object).flatMap(([k, v]) => leaves(v, path ? `${path}.${k}` : k));
}

// Values that are the same word in both languages: product and brand names, commands.
const sameInBoth = new Set(["GitHub", "apt", "checksum", "Changelog"]);

it("pt-BR has every English key and translates every value", () => {
  const en = leaves(dicts.en);
  const pt = new Map(leaves(dicts["pt-BR"]));
  expect([...pt.keys()]).toEqual(en.map(([k]) => k));
  const untranslated = en.filter(([k, v]) => pt.get(k) === v && !sameInBoth.has(v)).map(([k]) => k);
  expect(untranslated).toEqual([]);
});

it.each(Object.entries(dicts))("%s meta description is 140 to 160 characters", (_, t: Dict) => {
  expect(t.meta.description.length).toBeGreaterThanOrEqual(140);
  expect(t.meta.description.length).toBeLessThanOrEqual(160);
});

it("formats numbers and dates per locale", () => {
  expect(formatNumber("en", 100000)).toBe("100 000");
  expect(formatNumber("en", 0.58)).toBe("0.58");
  expect(formatNumber("pt-BR", 100000)).toBe("100.000");
  expect(formatNumber("pt-BR", 0.58)).toBe("0,58");
  expect(formatDate("en", "2026-11-14")).toBe("November 14, 2026");
  expect(formatDate("pt-BR", "2026-11-14")).toBe("14 de novembro de 2026");
});

it("renders the Portuguese page in Portuguese", () => {
  render(<App lang="pt-BR" />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(dicts["pt-BR"].hero.headline);
  expect(screen.getByText(/100\.000 linhas/)).toBeTruthy();
  const nav = screen.getByRole("navigation", { name: "Principal" });
  const english = within(nav).getByRole("link", { name: "English" });
  expect(english.getAttribute("href")).toBe("/");
  expect(english.getAttribute("hreflang")).toBe("en");
  expect(english.getAttribute("lang")).toBe("en");
  expect(screen.getAllByRole("link", { name: "hoprs, início" })[0].getAttribute("href")).toBe("/pt-br/");
});

it("links the English page to the Portuguese one", () => {
  render(<App />);
  const link = within(screen.getByRole("navigation", { name: "Primary" })).getByRole("link", { name: "Português" });
  expect(link.getAttribute("href")).toBe("/pt-br/");
  expect(link.getAttribute("hreflang")).toBe("pt-BR");
  expect(link.getAttribute("lang")).toBe("pt-BR");
});

it("OsLogo is a decorative SVG in the text colour", () => {
  const { container } = render(<OsLogo os="windows" />);
  const svg = container.querySelector("svg")!;
  expect(svg.getAttribute("aria-hidden")).toBe("true");
  expect(svg.getAttribute("fill")).toBe("currentColor");
  expect(svg.querySelector("path")?.getAttribute("d")).toBeTruthy();
});
