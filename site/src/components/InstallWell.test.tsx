import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { InstallWell } from "./InstallWell";
import { facts } from "../facts";

const tabs = [
  { id: "apt", label: "apt", lines: [`wget ${facts.debUrl}`, `sudo apt install ./${facts.debName}`] },
  { id: "checksum", label: "checksum", lines: [`wget ${facts.sumsUrl}`, "sha256sum -c SHA256SUMS --ignore-missing"] },
  { id: "source", label: "source", lines: ["git clone x", "cd hoprs"] },
];
const aptText = tabs[0].lines.join("\n");

function setClipboard(value: unknown) {
  Object.defineProperty(navigator, "clipboard", { value, configurable: true });
}

beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setClipboard(undefined);
});

it("tabs follow the ARIA tabs pattern", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  render(<InstallWell tabs={tabs} />);
  expect(screen.getByRole("tablist")).toBeTruthy();
  const [apt, checksum, source] = screen.getAllByRole("tab");
  expect(apt.getAttribute("aria-selected")).toBe("true");
  apt.focus();
  await user.keyboard("{ArrowRight}");
  expect(checksum.getAttribute("aria-selected")).toBe("true");
  expect(apt.getAttribute("aria-selected")).toBe("false");
  expect(document.activeElement).toBe(checksum);
  await user.keyboard("{ArrowLeft}{ArrowLeft}");
  expect(source.getAttribute("aria-selected")).toBe("true");
  const panel = screen.getByRole("tabpanel");
  expect(panel.getAttribute("aria-labelledby")).toBe(source.id);
});

it("copy writes all lines joined by newline", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  setClipboard({ writeText });
  render(<InstallWell tabs={tabs} />);
  await user.click(screen.getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith(aptText);
  expect(screen.getByRole("button", { name: "Copied" }).dataset.state).toBe("copied");
  expect(screen.getByRole("status").textContent).toBe("Copied");
  act(() => {
    vi.advanceTimersByTime(1400);
  });
  expect(screen.getByRole("button", { name: "Copy" }).dataset.state).toBeUndefined();
});

it("falls back to selecting the text", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  setClipboard({ writeText: vi.fn().mockRejectedValue(new Error("denied")) });
  render(<InstallWell tabs={tabs} />);
  await user.click(screen.getByRole("button", { name: "Copy" }));
  expect(window.getSelection()?.toString()).toBe(aptText);
  expect(screen.getByRole("status").textContent).toBe("Press Ctrl+C to copy");
});

it("keeps the Ctrl+C notice until the next interaction", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  setClipboard(undefined);
  render(<InstallWell tabs={tabs} />);
  await user.click(screen.getByRole("button", { name: "Copy" }));
  act(() => {
    vi.advanceTimersByTime(10_000);
  });
  expect(screen.getByRole("status").textContent).toBe("Press Ctrl+C to copy");
  await user.keyboard("{Control>}c{/Control}");
  expect(screen.getByRole("status").textContent).toBe("");
});

it("falls back when the clipboard API is missing", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  setClipboard(undefined);
  render(<InstallWell tabs={tabs} />);
  await user.click(screen.getByRole("button", { name: "Copy" }));
  expect(window.getSelection()?.toString()).toBe(aptText);
});

it("renders the prompt glyph apart from the command text", () => {
  const { container } = render(<InstallWell tabs={tabs} />);
  const prompts = container.querySelector(".install-prompts");
  expect(prompts?.getAttribute("aria-hidden")).toBe("true");
  expect(prompts?.textContent).toContain("$");
  expect(container.querySelector("code")?.textContent).toBe(aptText);
});
