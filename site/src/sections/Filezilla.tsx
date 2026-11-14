type Kind = " " | "+" | "@";
const kindClass: Record<Kind, string> = { " ": "diff-ctx", "+": "diff-add", "@": "diff-meta" };

const lines: { kind: Kind; text: string }[] = [
  { kind: "@", text: "@@ sitemanager.xml @@" },
  { kind: " ", text: "saved sites, one click to open" },
  { kind: "+", text: "imports sitemanager.xml, with a report of what was skipped" },
  { kind: "+", text: "passwords in the system keyring" },
  { kind: "+", text: "one encrypted backup file" },
  { kind: "+", text: "terminal and files over one connection" },
];

export default function Filezilla() {
  return (
    <section className="section container" aria-labelledby="fz-title">
      <h2 id="fz-title">Your FileZilla sites, kept.</h2>
      <pre className="well diff" aria-label="What changes when you move from FileZilla to hoprs">
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
