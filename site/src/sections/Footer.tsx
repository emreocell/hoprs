import { facts } from "../facts";

export default function Footer() {
  return (
    <footer className="footer">
      <div className="container footer-row">
        <a className="wordmark" href="/" aria-label="hoprs, home">
          <img src="/icon.svg" alt="" width="24" height="24" />
          <span>hoprs</span>
        </a>
        <nav aria-label="Footer" className="nav-links">
          <a href={facts.repoUrl}>GitHub</a>
          <a href={facts.releasesUrl}>Releases</a>
          <a href={facts.changelogUrl}>Changelog</a>
          <a href={facts.securityUrl}>Security</a>
          <a href={facts.licenseUrl}>License</a>
        </nav>
        <p className="muted small">Licensed MIT OR Apache-2.0.</p>
      </div>
    </footer>
  );
}
