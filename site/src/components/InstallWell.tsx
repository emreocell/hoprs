import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { useT } from "../i18n";

export type InstallTab = { id: string; label: string; lines: string[] };
type Notice = "" | "copied" | "pressCtrlC";

// "Copied" fades after a moment; the Ctrl+C hint stays until the next key press or click.
const RESET_MS = 1400;

export function InstallWell({ tabs }: { tabs: InstallTab[] }) {
  const uid = useId();
  const t = useT().install;
  const [active, setActive] = useState(tabs[0]?.id ?? "");
  const [notice, setNotice] = useState<Notice>("");
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const tabRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const codeRefs = useRef<Record<string, HTMLElement | null>>({});

  useEffect(() => () => clearTimeout(timer.current), []);

  const fallback = notice === "pressCtrlC";
  useEffect(() => {
    if (!fallback) return;
    const clear = () => setNotice("");
    document.addEventListener("keydown", clear);
    document.addEventListener("pointerdown", clear);
    return () => {
      document.removeEventListener("keydown", clear);
      document.removeEventListener("pointerdown", clear);
    };
  }, [fallback]);

  const announce = (next: Notice) => {
    setNotice(next);
    clearTimeout(timer.current);
    if (next === "copied") timer.current = setTimeout(() => setNotice(""), RESET_MS);
  };

  const select = (id: string) => {
    setActive(id);
    setNotice("");
    clearTimeout(timer.current);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let next = -1;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    if (next < 0) return;
    event.preventDefault();
    select(tabs[next].id);
    tabRefs.current[tabs[next].id]?.focus();
  };

  const copy = async () => {
    const tab = tabs.find((t) => t.id === active);
    if (!tab) return;
    try {
      await navigator.clipboard.writeText(tab.lines.join("\n"));
      announce("copied");
    } catch {
      const code = codeRefs.current[tab.id];
      const selection = window.getSelection();
      if (code && selection) {
        const range = document.createRange();
        range.selectNodeContents(code);
        selection.removeAllRanges();
        selection.addRange(range);
      }
      announce("pressCtrlC");
    }
  };

  const copied = notice === "copied";

  return (
    <div className="install-well">
      <div className="install-bar">
        <div role="tablist" aria-label={t.method} className="install-tabs">
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              ref={(el) => {
                tabRefs.current[tab.id] = el;
              }}
              type="button"
              role="tab"
              id={`${uid}-tab-${tab.id}`}
              aria-selected={tab.id === active}
              aria-controls={`${uid}-panel-${tab.id}`}
              tabIndex={tab.id === active ? 0 : -1}
              className="install-tab"
              onClick={() => select(tab.id)}
              onKeyDown={(e) => onKeyDown(e, index)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <button type="button" className="install-copy" data-state={copied ? "copied" : undefined} onClick={copy}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
            {copied ? (
              <path d="M3 8.5l3.5 3.5L13 4.5" />
            ) : (
              <>
                <rect x="5.5" y="5.5" width="8" height="8" rx="1" />
                <path d="M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2" />
              </>
            )}
          </svg>
          <span>{copied ? t.copied : t.copy}</span>
        </button>
      </div>
      {tabs.map((tab) => (
        <div
          key={tab.id}
          role="tabpanel"
          id={`${uid}-panel-${tab.id}`}
          aria-labelledby={`${uid}-tab-${tab.id}`}
          hidden={tab.id !== active}
        >
          <pre className="well cmd install-pre">
            <span className="install-prompts" aria-hidden="true">
              {tab.lines.map((_, i) => (
                <span key={i}>$</span>
              ))}
            </span>
            <code
              ref={(el) => {
                codeRefs.current[tab.id] = el;
              }}
            >
              {tab.lines.join("\n")}
            </code>
          </pre>
        </div>
      ))}
      <p role="status" className="install-status small muted">{notice ? t[notice] : ""}</p>
    </div>
  );
}
