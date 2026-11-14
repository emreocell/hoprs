import { facts } from "./facts";
import type { Os } from "./platform";

/** The main download per system, in the order the page lists them when nothing is detected. */
export const systems: readonly Os[] = ["linux", "macos", "windows"];

export const downloads: Record<Os, { name: string; ext: string; href: string }> = {
  linux: { name: "Linux", ext: ".deb", href: facts.debUrl },
  macos: { name: "macOS", ext: ".dmg", href: facts.dmgUrl },
  windows: { name: "Windows", ext: ".msi", href: facts.msiUrl },
};

/** The detected system first, then the others in the usual order. */
export function downloadOrder(first: Os): Os[] {
  return [first, ...systems.filter((os) => os !== first)];
}
