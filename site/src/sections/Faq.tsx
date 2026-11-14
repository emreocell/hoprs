export const faq = [
  { q: "Does it run on Windows or macOS?", a: "Not packaged yet; the code is portable." },
  { q: "Where are passwords stored?", a: "In the system keyring (Secret Service). Site files hold no secrets." },
  { q: "Is plain FTP allowed?", a: "Yes. It is labelled ftp (unencrypted). FTPS, explicit and implicit, is supported too." },
  { q: "Does it import from FileZilla?", a: "Yes. It reads sitemanager.xml and reports what was imported and what was skipped." },
  { q: "Does it collect data?", a: "No. It has no accounts and talks only to the servers you add." },
  { q: "How do I uninstall it?", a: "Run sudo apt remove hoprs. Your data stays; delete ~/.config/hoprs and ~/.local/share/hoprs to remove it too." },
  { q: "How do I report a bug?", a: "Open an issue on GitHub. Report vulnerabilities privately through GitHub security advisories." },
];

export default function Faq() {
  return (
    <section className="section container" aria-labelledby="faq-title">
      <h2 id="faq-title">Questions.</h2>
      <div className="faq">
        {faq.map(({ q, a }) => (
          <details key={q}>
            <summary>{q}</summary>
            <p>{a}</p>
          </details>
        ))}
      </div>
    </section>
  );
}
