import { facts } from "../facts";
import OsLogo from "../components/OsLogo";
import Shot from "../components/Shot";
import { downloadOrder, downloads } from "../downloads";
import { GlyphField } from "../glyph/GlyphField";
import { useT } from "../i18n";
import { useDetectedOs } from "../platform";

export default function Hero() {
  const t = useT().hero;
  // The prerender lists Linux first; after hydration the visitor's system moves to the front.
  const order = downloadOrder(useDetectedOs());
  return (
    <>
      <section className="hero" aria-labelledby="hero-title">
        <div data-slot="glyph-field" className="glyph-field" aria-hidden="true">
          <GlyphField />
        </div>
        <div className="container intro">
          <h1 id="hero-title">{t.headline}</h1>
          <p className="lead">{t.sentence}</p>
          <div className="actions">
            {order.map((os, i) => {
              const d = downloads[os];
              return (
                <a key={os} className={`btn ${i === 0 ? "btn-primary" : "btn-secondary"}`} href={d.href}>
                  <OsLogo os={os} />
                  <span className="sr-only">{t.downloadFor}</span>{" "}
                  {d.name} {d.ext}
                </a>
              );
            })}
            <a className="btn btn-secondary" href={facts.repoUrl}>GitHub</a>
          </div>
        </div>
      </section>
      {/* Below the hero, not in it: the glyph field never runs behind the opaque shot. */}
      <div className="container hero-shot">
        <Shot name="main" slot="shot-main" alt={t.shotAlt} />
      </div>
    </>
  );
}
