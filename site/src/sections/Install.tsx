import { useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { InstallWell } from "../components/InstallWell";
import { facts } from "../facts";
import { useDetectedOs } from "../platform";
import type { Os } from "../platform";

const linuxTabs = [
  { id: "apt", label: "apt", lines: [`wget ${facts.debUrl}`, `sudo apt install ./${facts.debName}`] },
  { id: "checksum", label: "checksum", lines: [`wget ${facts.sumsUrl}`, "sha256sum -c SHA256SUMS --ignore-missing"] },
  { id: "source", label: "source", lines: [`git clone ${facts.repoUrl}.git`, "cd hoprs", "cargo run --release"] },
];

const systems: { id: Os; label: string }[] = [
  { id: "linux", label: "Linux" },
  { id: "macos", label: "macOS" },
  { id: "windows", label: "Windows" },
];

export default function Install() {
  const uid = useId();
  const detected = useDetectedOs();
  const [picked, setPicked] = useState<Os | null>(null);
  const active = picked ?? detected;
  const tabRefs = useRef<Record<string, HTMLButtonElement | null>>({});

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let next = -1;
    if (event.key === "ArrowRight") next = (index + 1) % systems.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + systems.length) % systems.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = systems.length - 1;
    if (next < 0) return;
    event.preventDefault();
    setPicked(systems[next].id);
    tabRefs.current[systems[next].id]?.focus();
  };

  return (
    <section id="install" className="section container" aria-labelledby="install-title">
      <h2 id="install-title">Ready when you are.</h2>
      <div className="install-systems">
        <div role="tablist" aria-label="Operating system" className="install-tabs">
          {systems.map((system, index) => (
            <button
              key={system.id}
              ref={(el) => {
                tabRefs.current[system.id] = el;
              }}
              type="button"
              role="tab"
              id={`${uid}-os-${system.id}`}
              aria-selected={system.id === active}
              aria-controls={`${uid}-os-panel-${system.id}`}
              tabIndex={system.id === active ? 0 : -1}
              className="install-tab"
              onClick={() => setPicked(system.id)}
              onKeyDown={(e) => onKeyDown(e, index)}
            >
              {system.label}
            </button>
          ))}
        </div>
      </div>
      <div
        role="tabpanel"
        className="os-panel"
        id={`${uid}-os-panel-linux`}
        aria-labelledby={`${uid}-os-linux`}
        hidden={active !== "linux"}
      >
        <p className="lead">Ubuntu 24.04 or newer, amd64.</p>
        <InstallWell tabs={linuxTabs} />
      </div>
      <div
        role="tabpanel"
        className="os-panel"
        id={`${uid}-os-panel-macos`}
        aria-labelledby={`${uid}-os-macos`}
        hidden={active !== "macos"}
      >
        <p className="lead">macOS 11 or newer, Apple silicon and Intel.</p>
        <p>
          <a className="btn btn-primary" href={facts.dmgUrl}>Download {facts.dmgName}</a>
        </p>
        <ol className="steps">
          <li>Open the disk image and drag hoprs to Applications.</li>
          <li>The app is not notarized. On first launch, right-click hoprs and choose Open, then Open again.</li>
          <li>If macOS still blocks it, open System Settings, Privacy &amp; Security, and choose Open Anyway.</li>
        </ol>
      </div>
      <div
        role="tabpanel"
        className="os-panel"
        id={`${uid}-os-panel-windows`}
        aria-labelledby={`${uid}-os-windows`}
        hidden={active !== "windows"}
      >
        <p className="lead">Windows, 64-bit.</p>
        <p className="actions">
          <a className="btn btn-primary" href={facts.msiUrl}>Download {facts.msiName}</a>
          <a className="btn btn-secondary" href={facts.zipUrl}>Portable {facts.zipName}</a>
        </p>
        <ol className="steps">
          <li>Run the installer, or unzip the portable build anywhere.</li>
          <li>The app is not signed. If SmartScreen says Windows protected your PC, choose More info, then Run anyway.</li>
        </ol>
      </div>
    </section>
  );
}
