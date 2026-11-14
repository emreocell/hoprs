import { facts } from "../facts";
import Shot from "../components/Shot";
import { GlyphField } from "../glyph/GlyphField";

export const heroHeadline = "SSH, SFTP and FTP in one window.";
export const heroSentence = "A fast, native file client for Linux, written in Rust.";

export default function Hero() {
  return (
    <>
      <section className="hero" aria-labelledby="hero-title">
        <div data-slot="glyph-field" className="glyph-field" aria-hidden="true">
          <GlyphField />
        </div>
        <div className="container intro">
          <h1 id="hero-title">{heroHeadline}</h1>
          <p className="lead">{heroSentence}</p>
          <div className="actions">
            <a className="btn btn-primary" href={facts.debUrl}>Download .deb</a>
            <a className="btn btn-secondary" href={facts.repoUrl}>GitHub</a>
          </div>
        </div>
      </section>
      {/* Below the hero, not in it: the glyph field never runs behind the opaque shot. */}
      <div className="container hero-shot">
        <Shot
          name="main"
          slot="shot-main"
          alt="hoprs files tab: the local project folder on the left, the same folder on the build server on the right, one upload running in the transfer queue."
        />
      </div>
    </>
  );
}
