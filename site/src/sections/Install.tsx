import { InstallWell } from "../components/InstallWell";
import { facts } from "../facts";

const tabs = [
  { id: "apt", label: "apt", lines: [`wget ${facts.debUrl}`, `sudo apt install ./${facts.debName}`] },
  { id: "checksum", label: "checksum", lines: [`wget ${facts.sumsUrl}`, "sha256sum -c SHA256SUMS --ignore-missing"] },
  { id: "source", label: "source", lines: [`git clone ${facts.repoUrl}.git`, "cd hoprs", "cargo run --release"] },
];

export default function Install() {
  return (
    <section id="install" className="section container" aria-labelledby="install-title">
      <h2 id="install-title">Ready when you are.</h2>
      <p className="lead">Ubuntu 24.04 or newer, amd64.</p>
      <InstallWell tabs={tabs} />
    </section>
  );
}
