import { facts } from "../facts";

const { perf } = facts;
const rows = [
  { value: `${perf.startupMs} ms`, note: "to the first frame" },
  { value: `${perf.idleMb} MB`, note: "memory at idle" },
  { value: `${perf.rowsP95Ms} ms`, note: `p95 view and layout, ${perf.rows.toLocaleString("en-US").replaceAll(",", "\u00a0")} rows` },
  { value: `${perf.sftpVsScp} x scp`, note: "SFTP, 1 GiB at 50 ms RTT" },
];

export default function Fast() {
  return (
    <section className="section container" aria-labelledby="fast-title">
      <h2 id="fast-title">Light on memory, quick on the wire.</h2>
      <ul className="well out" aria-label="Measured performance">
        {rows.map((r) => (
          <li key={r.value}>
            <span className="out-value num">{r.value}</span>
            <span className="out-note"># {r.note}</span>
          </li>
        ))}
      </ul>
      <p className="muted small">
        Measured on a GitHub Actions runner, {perf.measuredOn}.{" "}
        <a href={facts.perfDocUrl}>How it is measured</a>
      </p>
    </section>
  );
}
