import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import App from "../App";
import { facts } from "../facts";
import Install from "./Install";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function stubUa(ua: string, platform = "") {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);
  vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
}

it("offers three system tabs, Linux first", () => {
  render(<Install />);
  const tabs = screen.getAllByRole("tab", { name: /^(Linux|macOS|Windows)$/ });
  expect(tabs.map((t) => t.textContent)).toEqual(["Linux", "macOS", "Windows"]);
  expect(tabs[0].getAttribute("aria-selected")).toBe("true");
});

it("macOS tab has the .dmg link and the Gatekeeper steps", async () => {
  const user = userEvent.setup();
  render(<Install />);
  await user.click(screen.getByRole("tab", { name: "macOS" }));
  const panel = screen.getByRole("tabpanel", { name: "macOS" });
  expect(within(panel).getByRole("link", { name: /\.dmg/ }).getAttribute("href")).toBe(facts.dmgUrl);
  expect(panel.textContent).toContain("Open Anyway");
});

it("Windows tab has .msi and .zip links and the SmartScreen step", async () => {
  const user = userEvent.setup();
  render(<Install />);
  await user.click(screen.getByRole("tab", { name: "Windows" }));
  const panel = screen.getByRole("tabpanel", { name: "Windows" });
  expect(within(panel).getByRole("link", { name: /\.msi/ }).getAttribute("href")).toBe(facts.msiUrl);
  expect(within(panel).getByRole("link", { name: /\.zip/ }).getAttribute("href")).toBe(facts.zipUrl);
  expect(panel.textContent).toContain("SmartScreen");
});

it("the hero button follows the detected system after hydration", () => {
  stubUa("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "MacIntel");
  render(<App />);
  const hero = screen.getByRole("heading", { level: 1 }).closest("section")!;
  expect(within(hero).getByRole("link", { name: "Download .dmg" }).getAttribute("href")).toBe(facts.dmgUrl);
});

it("the hero button is the .deb on Linux", () => {
  stubUa("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64");
  render(<App />);
  const hero = screen.getByRole("heading", { level: 1 }).closest("section")!;
  expect(within(hero).getByRole("link", { name: "Download .deb" }).getAttribute("href")).toBe(facts.debUrl);
});
