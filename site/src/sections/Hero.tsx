import { facts } from "../facts";

export const heroHeadline = "SSH, SFTP and FTP in one window.";
export const heroSentence = "A fast, native file client for Linux, written in Rust.";

export default function Hero() {
  return (
    <section className="hero" aria-labelledby="hero-title">
      <div data-slot="glyph-field" className="glyph-field" aria-hidden="true" />
      <div className="container hero-body">
        <h1 id="hero-title">{heroHeadline}</h1>
        <p className="lead">{heroSentence}</p>
        <div className="actions">
          <a className="btn btn-primary" href={facts.debUrl}>Download .deb</a>
          <a className="btn btn-secondary" href={facts.repoUrl}>GitHub</a>
        </div>
        <figure data-slot="shot-main" className="shot" />
      </div>
    </section>
  );
}
