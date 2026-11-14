import Shot from "../components/Shot";
import { useT } from "../i18n";

export default function Connection() {
  const t = useT().connection;
  return (
    <section className="section container" aria-labelledby="conn-title">
      <h2 id="conn-title">{t.title}</h2>
      <p className="lead">{t.lead}</p>
      <Shot name="split" slot="shot-split" alt={t.shotAlt} lazy />
    </section>
  );
}
