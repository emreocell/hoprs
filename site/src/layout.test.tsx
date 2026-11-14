import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";

import Install from "./sections/Install";

const css = readFileSync(resolve(process.cwd(), "src/styles/app.css"), "utf8");
const rule = (selector: string) => {
  const m = css.match(new RegExp(`(?:^|\\n)${selector.replace(".", "\\.")}\\s*\\{([^}]*)\\}`));
  return m?.[1] ?? "";
};

afterEach(cleanup);

it("install well never widens the page", () => {
  const { container } = render(<Install />);
  const pre = container.querySelector("pre")!;
  expect(pre.classList.contains("install-pre")).toBe(true);
  expect(rule(".install-pre")).toMatch(/overflow-x:\s*auto/);
  expect(rule(".install-pre")).toMatch(/max-width:\s*100%/);
  expect(rule(".section")).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  expect(rule(".install-well")).toMatch(/min-width:\s*0/);
});
