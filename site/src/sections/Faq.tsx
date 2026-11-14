import { useT } from "../i18n";

export default function Faq() {
  const t = useT().faq;
  return (
    <section className="section container" aria-labelledby="faq-title">
      <h2 id="faq-title">{t.title}</h2>
      <div className="faq">
        {t.items.map(({ q, a }) => (
          <details key={q}>
            <summary>{q}</summary>
            <p>{a}</p>
          </details>
        ))}
      </div>
    </section>
  );
}
