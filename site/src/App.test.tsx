import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import App from "./App";

it("renders a main landmark with a level 1 heading", () => {
  render(<App />);
  expect(screen.getByRole("main")).toBeTruthy();
  expect(screen.getByRole("heading", { level: 1 })).toBeTruthy();
});
