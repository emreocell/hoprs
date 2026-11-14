import { facts } from "../facts";

// Task 3 replaces this placeholder with <InstallWell />.
export default function Install() {
  return (
    <section id="install" className="section container" aria-labelledby="install-title">
      <h2 id="install-title">Ready when you are.</h2>
      <p className="lead">Ubuntu 24.04 or newer, amd64.</p>
      <pre className="well cmd">
        <code>{`wget ${facts.debUrl}\nsudo apt install ./${facts.debName}`}</code>
      </pre>
    </section>
  );
}
