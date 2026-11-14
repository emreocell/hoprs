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

function heroDownloads() {
  const hero = screen.getByRole("heading", { level: 1 }).closest("section")!;
  return within(hero)
    .getAllByRole("link")
    .filter((a) => a.getAttribute("href")!.includes("/releases/download/"));
}

it("the hero offers all three downloads, the detected system first and primary", () => {
  stubUa("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "MacIntel");
  render(<App />);
  const links = heroDownloads();
  expect(links.map((a) => a.getAttribute("href"))).toEqual([facts.dmgUrl, facts.debUrl, facts.msiUrl]);
  expect(links.map((a) => a.className)).toEqual(["btn btn-primary", "btn btn-secondary", "btn btn-secondary"]);
  expect(links[0].textContent).toBe("Download for macOS .dmg");
  for (const a of links) expect(a.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
});

it("the hero lists Linux first on Linux, and Windows first on Windows", () => {
  stubUa("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64");
  render(<App />);
  expect(heroDownloads().map((a) => a.getAttribute("href"))).toEqual([facts.debUrl, facts.dmgUrl, facts.msiUrl]);
  expect(screen.getByRole("link", { name: "Download for Linux .deb" }).className).toBe("btn btn-primary");
  cleanup();
  stubUa("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "Win32");
  render(<App />);
  expect(heroDownloads().map((a) => a.getAttribute("href"))).toEqual([facts.msiUrl, facts.debUrl, facts.dmgUrl]);
});

it("the system tabs carry the system marks", () => {
  render(<Install />);
  for (const tab of screen.getAllByRole("tab", { name: /^(Linux|macOS|Windows)$/ })) {
    expect(tab.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  }
});
