export const faq = [
  { q: "Which systems does it run on?", a: "Linux (Ubuntu 24.04 or newer, amd64), macOS 11 or newer, and 64-bit Windows." },
  { q: "macOS or Windows says the app is not trusted. Is that expected?", a: "Yes. The builds are not notarized or signed with a paid certificate. On macOS, right-click hoprs, choose Open, then Open again; or use Open Anyway in System Settings, Privacy & Security. On Windows, choose More info, then Run anyway. Check the download against SHA256SUMS first." },
  { q: "Where are passwords stored?", a: "In the system keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. Site files hold no secrets." },
  { q: "Is plain FTP allowed?", a: "Yes. It is labelled ftp (unencrypted). FTPS, explicit and implicit, is supported too." },
  { q: "Does it import from FileZilla?", a: "Yes. It reads sitemanager.xml and reports what was imported and what was skipped." },
  { q: "Does it collect data?", a: "No. It has no accounts and talks only to the servers you add." },
  { q: "How do I uninstall it?", a: "Linux: sudo apt remove hoprs. macOS: drag hoprs out of Applications. Windows: remove it in Installed apps. Your data stays; to remove it too, delete ~/.config/hoprs and ~/.local/share/hoprs (Linux), ~/Library/Application Support/hoprs (macOS) or %APPDATA%\\hoprs (Windows), and the keyring entries whose service starts with hoprs/." },
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
