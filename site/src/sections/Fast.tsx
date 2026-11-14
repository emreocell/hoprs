import { facts } from "../facts";
import { formatDate, formatNumber, useLang, useT } from "../i18n";

const { perf } = facts;

export default function Fast() {
  const lang = useLang();
  const t = useT().fast;
  const n = (value: number) => formatNumber(lang, value);
  const rows = [
    { value: `${n(perf.startupMs)} ms`, note: t.startup },
    { value: `${n(perf.idleMb)} MB`, note: t.idle },
    { value: `${n(perf.rowsP95Ms)} ms`, note: t.rows(n(perf.rows)) },
    { value: `${n(perf.sftpVsScp)} x scp`, note: t.sftp },
  ];
  return (
    <section className="section container" aria-labelledby="fast-title">
      <h2 id="fast-title">{t.title}</h2>
      <ul className="well out" aria-label={t.label}>
        {rows.map((r) => (
          <li key={r.value}>
            <span className="out-value num">{r.value}</span>
            <span className="out-note"># {r.note}</span>
          </li>
        ))}
      </ul>
      <p className="muted small">
        {t.measuredOn} <time dateTime={perf.measuredOn}>{formatDate(lang, perf.measuredOn)}</time>.{" "}
        <a href={facts.perfDocUrl}>{t.how}</a>
      </p>
    </section>
  );
}
