import { facts } from "../facts";
import { otherLang, pagePath, useLang, useT } from "../i18n";

export default function Nav() {
  const lang = useLang();
  const t = useT().nav;
  const other = otherLang(lang);
  return (
    <header className="nav">
      <div className="container nav-row">
        <a className="wordmark" href={pagePath[lang]} aria-label={t.home}>
          <img src="/icon.svg" alt="" width="28" height="28" />
          <span>hoprs</span>
        </a>
        <nav aria-label={t.label} className="nav-links">
          <a href={facts.repoUrl}>GitHub</a>
          <a href="#install">{t.download}</a>
          <a href={pagePath[other]} hrefLang={other} lang={other}>
            {t.otherLanguage}
          </a>
          <span className="muted nav-version">{facts.version}</span>
        </nav>
      </div>
    </header>
  );
}
