import { useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { InstallWell } from "../components/InstallWell";
import OsLogo from "../components/OsLogo";
import { downloads, systems as systemIds } from "../downloads";
import { facts } from "../facts";
import { useT } from "../i18n";
import { useDetectedOs } from "../platform";
import type { Os } from "../platform";

const systems: { id: Os; label: string }[] = systemIds.map((id) => ({ id, label: downloads[id].name }));

export default function Install() {
  const uid = useId();
  const t = useT().install;
  const linuxTabs = [
    { id: "apt", label: "apt", lines: [`wget ${facts.debUrl}`, `sudo apt install ./${facts.debName}`] },
    { id: "checksum", label: "checksum", lines: [`wget ${facts.sumsUrl}`, "sha256sum -c SHA256SUMS --ignore-missing"] },
    { id: "source", label: t.source, lines: [`git clone ${facts.repoUrl}.git`, "cd hoprs", "cargo run --release"] },
  ];
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
      <h2 id="install-title">{t.title}</h2>
      <div className="install-systems">
        <div role="tablist" aria-label={t.systems} className="install-tabs">
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
              <OsLogo os={system.id} />
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
        <p className="lead">{t.linuxLead}</p>
        <InstallWell tabs={linuxTabs} />
      </div>
      <div
        role="tabpanel"
        className="os-panel"
        id={`${uid}-os-panel-macos`}
        aria-labelledby={`${uid}-os-macos`}
        hidden={active !== "macos"}
      >
        <p className="lead">{t.macLead}</p>
        <p>
          <a className="btn btn-primary" href={facts.dmgUrl}>
            {t.download} {facts.dmgName}
          </a>
        </p>
        <ol className="steps">
          {t.macSteps.map((step) => (
            <li key={step}>{step}</li>
          ))}
          <li>
            {t.macTerminal} <code>xattr -dr com.apple.quarantine /Applications/hoprs.app</code>
          </li>
        </ol>
        <p className="small muted">
          {t.checkWith} <a href={facts.sumsUrl}>SHA256SUMS</a>.
        </p>
      </div>
      <div
        role="tabpanel"
        className="os-panel"
        id={`${uid}-os-panel-windows`}
        aria-labelledby={`${uid}-os-windows`}
        hidden={active !== "windows"}
      >
        <p className="lead">{t.windowsLead}</p>
        <p className="actions">
          <a className="btn btn-primary" href={facts.msiUrl}>
            {t.download} {facts.msiName}
          </a>
          <a className="btn btn-secondary" href={facts.zipUrl}>
            {t.portable} {facts.zipName}
          </a>
        </p>
        <ol className="steps">
          {t.windowsSteps.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
        <p className="small muted">
          {t.checkWith} <a href={facts.sumsUrl}>SHA256SUMS</a>.
        </p>
      </div>
    </section>
  );
}
