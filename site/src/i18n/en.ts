// English copy: the default page at /. pt-br.ts has the same shape (typed as Dict), so a
// missing or extra key fails typecheck. Facts (version, URLs, numbers) live in facts.ts.
export const en = {
  meta: {
    title: "hoprs: SSH, SFTP and FTP client for Linux, macOS and Windows",
    description:
      "hoprs is a free, open source SSH, SFTP, FTP and FTPS client with a built-in terminal. It imports your FileZilla sites. For Linux, macOS and Windows.",
    ogLocale: "en_US",
    ogImageAlt: "hoprs, SSH, SFTP and FTP in one window: the files tab with a local folder and the same folder on a server.",
  },
  nav: {
    label: "Primary",
    home: "hoprs, home",
    download: "Download",
    // The link to the other language, written in that language.
    otherLanguage: "Português",
  },
  hero: {
    headline: "SSH, SFTP and FTP in one window.",
    sentence: "A fast, native file client for Linux, macOS and Windows.",
    downloadFor: "Download for",
    shotAlt:
      "hoprs files tab: two local files being dragged onto the build server's folder, which has a dashed outline and the label Upload 2 items to /home/alice/web-app; below, the folder comparison panel listing what differs between the two folders, and one upload running in the transfer queue.",
  },
  fast: {
    title: "Light on memory, quick on the wire.",
    label: "Measured performance",
    startup: "to the first frame",
    idle: "memory at idle",
    rows: (rows: string) => `p95 view and layout, ${rows} rows`,
    sftp: "SFTP, 1 GiB at 50 ms RTT",
    measuredOn: "Measured on a GitHub Actions runner,",
    how: "How it is measured",
  },
  connection: {
    title: "One connection, two tabs.",
    lead: "A terminal tab and a files tab share one SSH connection. Add more tabs over the same session.",
    shotAlt:
      "hoprs terminal tab on the same SSH connection, showing ls -la of the project folder on the server while the upload keeps running in the queue below.",
  },
  filezilla: {
    title: "Your FileZilla sites, kept.",
    label: "What changes when you move from FileZilla to hoprs",
    kept: "saved sites, one click to open",
    added: [
      "imports sitemanager.xml, with a report of what was skipped",
      "passwords in the system keyring",
      "one encrypted backup file",
      "terminal and files over one connection",
    ],
  },
  openSource: {
    title: "Open source.",
    lead: "MIT OR Apache-2.0. Built on iced, russh, suppaftp and alacritty_terminal.",
    link: "Read the code on GitHub",
  },
  install: {
    title: "Ready when you are.",
    systems: "Operating system",
    method: "Install method",
    source: "source",
    copy: "Copy",
    copied: "Copied",
    pressCtrlC: "Press Ctrl+C to copy",
    linuxLead: "Ubuntu 24.04 or newer, amd64.",
    macLead: "macOS 11 or newer, Apple silicon and Intel.",
    download: "Download",
    macSteps: [
      "Open the disk image and drag hoprs to Applications.",
      "The app is not notarized. Open hoprs once, then go to System Settings, Privacy & Security, and choose Open Anyway.",
    ],
    macTerminal: "Or, in Terminal:",
    checkWith: "Check it with",
    windowsLead: "Windows, 64-bit.",
    portable: "Portable",
    windowsSteps: [
      "Run the installer, or unzip the portable build anywhere.",
      "The app is not signed. If SmartScreen says Windows protected your PC, choose More info, then Run anyway.",
    ],
  },
  faq: {
    title: "Questions.",
    items: [
      { q: "Which systems does it run on?", a: "Linux (Ubuntu 24.04 or newer, amd64), macOS 11 or newer, and 64-bit Windows." },
      { q: "macOS or Windows says the app is not trusted. Is that expected?", a: "Yes. The builds are not notarized or signed with a paid certificate. On macOS, open System Settings, Privacy & Security, and choose Open Anyway; or run xattr -dr com.apple.quarantine /Applications/hoprs.app. On Windows, choose More info, then Run anyway. Check the download against SHA256SUMS first." },
      { q: "Where are passwords stored?", a: "In the system keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. Site files hold no secrets." },
      { q: "Is plain FTP allowed?", a: "Yes. It is labelled ftp (unencrypted). FTPS, explicit and implicit, is supported too." },
      { q: "Does it import from FileZilla?", a: "Yes. It reads sitemanager.xml and reports what was imported and what was skipped." },
      { q: "Does it collect data?", a: "The app: no. It has no accounts and talks only to the servers you add. This website counts visits with Cloudflare Web Analytics, which sets no cookies and stores no personal data." },
      { q: "How do I update it?", a: "hoprs does not update itself. Download the new version and install it over the old one: on Linux, sudo apt install ./hoprs_<version>-1_amd64.deb; on macOS, drag hoprs to Applications and replace it; on Windows, run the new .msi (it replaces the old version) or unzip the portable build over the old folder. Sites, workspaces, settings and saved passwords are kept." },
      { q: "How do I uninstall it?", a: "Linux: sudo apt remove hoprs. macOS: drag hoprs out of Applications. Windows: remove it in Installed apps. Your data stays; to remove it too, delete ~/.config/hoprs and ~/.local/share/hoprs (Linux), ~/Library/Application Support/hoprs (macOS) or %APPDATA%\\hoprs (Windows), and the keyring entries for service hoprs/<site-id> (shown as password.hoprs/<site-id> and key-passphrase.hoprs/<site-id> in the Windows Credential Manager)." },
      { q: "How do I report a bug?", a: "Open an issue on GitHub. Report vulnerabilities privately through GitHub security advisories." },
    ],
  },
  footer: {
    label: "Footer",
    releases: "Releases",
    changelog: "Changelog",
    security: "Security",
    license: "License",
    licensed: "Licensed MIT OR Apache-2.0.",
    madeBy: "Made by",
    updated: "Updated",
  },
};

export type Dict = typeof en;
