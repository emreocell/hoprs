import { facts } from "../facts";

export default function Nav() {
  return (
    <header className="nav">
      <div className="container nav-row">
        <a className="wordmark" href="/" aria-label="hoprs, home">
          <img src="/icon.svg" alt="" width="28" height="28" />
          <span>hoprs</span>
        </a>
        <nav aria-label="Primary" className="nav-links">
          <a href={facts.repoUrl}>GitHub</a>
          <a href="#install">Download</a>
          <span className="muted">{facts.version}</span>
        </nav>
      </div>
    </header>
  );
}
