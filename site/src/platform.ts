import { useSyncExternalStore } from "react";

export type Os = "linux" | "macos" | "windows";

// Unknown, phones and tablets fall back to Linux (the .deb), as the prerender does.
export function detectOs(ua: string, platform = ""): Os {
  if (/^mac/i.test(platform)) return "macos";
  if (/^win/i.test(platform)) return "windows";
  if (/Windows NT/i.test(ua)) return "windows";
  if (/iPhone|iPad|iPod|Android/i.test(ua)) return "linux";
  if (/Macintosh|Mac OS X/i.test(ua)) return "macos";
  return "linux";
}

function clientOs(): Os {
  const data = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  return detectOs(navigator.userAgent, data?.platform ?? navigator.platform);
}

const subscribe = () => () => {};

// The prerender cannot know the visitor, so the server snapshot is "linux" and the
// client switches right after hydration. That keeps the HTML deterministic and hydration clean.
export function useDetectedOs(): Os {
  return useSyncExternalStore(subscribe, clientOs, () => "linux");
}
