import { expect, it } from "vitest";
import { detectOs } from "./platform";

it("maps user agents and platforms to a system", () => {
  expect(detectOs("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")).toBe("macos");
  expect(detectOs("", "macOS")).toBe("macos");
  expect(detectOs("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/130")).toBe("windows");
  expect(detectOs("", "Win32")).toBe("windows");
  expect(detectOs("Mozilla/5.0 (X11; Linux x86_64) Firefox/130")).toBe("linux");
  expect(detectOs("")).toBe("linux");
  expect(detectOs("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)")).toBe("linux");
});
