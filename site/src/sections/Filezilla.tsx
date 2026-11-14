import { useT } from "../i18n";

type Kind = " " | "+" | "@";
const kindClass: Record<Kind, string> = { " ": "diff-ctx", "+": "diff-add", "@": "diff-meta" };

export default function Filezilla() {
  const t = useT().filezilla;
  const lines: { kind: Kind; text: string }[] = [
    { kind: "@", text: "@@ sitemanager.xml @@" },
    { kind: " ", text: t.kept },
    ...t.added.map((text) => ({ kind: "+" as const, text })),
  ];
  return (
    <section className="section container" aria-labelledby="fz-title">
      <h2 id="fz-title">{t.title}</h2>
      <pre className="well diff" aria-label={t.label}>
        <code>
          <span className="diff-meta">--- a/filezilla{"\n"}+++ b/hoprs{"\n"}</span>
          {lines.map((l) => (
            <span key={l.text} className={`diff-line ${kindClass[l.kind]}`}>
              {l.kind === "@" ? l.text : `${l.kind} ${l.text}`}
              {"\n"}
            </span>
          ))}
        </code>
      </pre>
    </section>
  );
}
