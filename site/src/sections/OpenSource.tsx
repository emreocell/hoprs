import { facts } from "../facts";
import { useT } from "../i18n";

export default function OpenSource() {
  const t = useT().openSource;
  return (
    <section className="section container" aria-labelledby="os-title">
      <h2 id="os-title">{t.title}</h2>
      <p className="lead">{t.lead}</p>
      <p>
        <a href={facts.repoUrl}>{t.link}</a>
      </p>
    </section>
  );
}
