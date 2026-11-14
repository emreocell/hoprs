# hoprs landing page — design

Date: 2026-11-14. Status: draft for the author's approval.

## 1. Intent

hoprs 1.0.0 is released and the repository is public. The landing page at
**https://hoprs.online** exists so that a Linux user who wants a better FileZilla lands, sees
in one screen what hoprs is, and leaves with the `.deb` installed or the GitHub repository
open. Success: a visitor goes from the hero to a working `apt install` without opening another
tab, and the page itself feels like the app (fast, quiet, technical).

The author asked for three things: follow https://wu.dstrml.org/ (its section structure and,
above all, the hero's cursor effect), use Hallmark with the "Terminal" theme modified to the
app's own look, and build it as a simple React + Tailwind page without Next.js.

### Goals

- One static page, English, that explains hoprs and drives two actions: **Download .deb**
  (primary) and **GitHub** (secondary).
- A hero whose background is a fluid simulation driven by the cursor, drawn as a grid of
  monospace characters (approach B below).
- The real app shown in real screenshots, and only real numbers.
- Deploy on the author's VPS behind Cloudflare, the same way the existing static site
  (`archviewer`) is deployed.

### Non-goals

- No docs site, blog, changelog page, i18n or language switcher (Wu has 中文; we do not).
- No analytics, cookies, forms, newsletter or accounts.
- No Windows or macOS downloads (not packaged; the FAQ says so).

### Anti-goals (a failure even if it works)

- Any invented metric, testimonial, logo wall, star count or comparison number hoprs did not
  measure. No numbers about FileZilla.
- A drawn or mocked-up app window instead of a real screenshot.
- A hero effect that makes the page slow: it must not hurt scrolling, must stop when off
  screen, and must not run at all with `prefers-reduced-motion`.
- A copy of Wu's look. The structure and the idea of the effect are borrowed; the material is
  hoprs'.

## 2. Approaches considered

**Hero effect** (the author's pick: **B**).

| | What | Trade-off |
|---|---|---|
| A | Wu's fluid as grey ink smoke behind the hero (WebGL2) | Closest to the reference, but it is Wu's effect with nothing of hoprs |
| **B** | The same fluid simulation, rendered as a grid of JetBrains Mono glyphs taken from hoprs' own vocabulary; under the cursor cells brighten toward white, then settle | Ties the reference to the app and the Terminal theme; more work; static grid without WebGL2 |
| C | CSS/canvas-2D glow and grid highlight | Light and simple, far from the reference |

**Stack.** Vite + React + TypeScript + Tailwind, static output. Next.js was considered and
dropped: one page, no server rendering, no routes, no data fetching. Astro would also fit
but the author asked for React + Tailwind. SEO needs only a correct `<head>` and the content
in the HTML, which we get by prerendering the single route at build time (see §6).

**Where the code lives.** A `site/` folder in this repository (chosen) versus a separate
repository. Same repo keeps the page next to the version it advertises (the build reads the
version and the perf numbers from the repo) and costs nothing in CI because the site job runs
only when `site/**` changes.

## 3. Look (Hallmark, "Terminal" modified)

Hallmark's Terminal theme supplies the **structure and material**: the page is a terminal
surface, square corners, 1px rules, no shadows, comparisons as diffs, figures printed like a
tool's output, install lines in a well with tabs and a copy button, a picture printed in
glyphs, stepped state changes, a caret that blinks five times and holds.

The app (mockup v2, `docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`) supplies
the **palette and type**, replacing Terminal's green phosphor:

- Neutral near-black, no hue: `--color-paper oklch(13% 0 0)`, `--color-paper-2 16.5%`,
  `--color-paper-3 21%`; ink `97%`, ink-2 `84%`, muted `63%`; rules `25%` and `34%`.
- The accent is **white** (`oklch(98% 0 0)`, accent ink `14%`); the accent wash is
  `oklch(24% 0 0)`. The glyph ramp (`--glyph-0..7`) is the same ramp in grey, from about 30%
  to 97% lightness.
- Type: **JetBrains Mono** for display heads, figures, commands, paths and the glyph field;
  **Inter** for body text, as in the app (UI in Inter, data in mono). This bends Terminal's
  "every glyph mono" rule on purpose, to match the app (the builder's call recorded here).
- Radius 4px where the app uses it (buttons, wells), otherwise square; 1px rules; no shadows.

Hallmark's Floor applies in full (tokens only, focus-visible on every control, reduced-motion
fallback, no eyebrows or tracked caps, no invented proof, hero visible at 1280 x 800, about 80
words per screen, capped container). `sloplint --render` must pass before the page ships.

## 4. Page structure (Wu's sections, mapped)

| # | Wu | hoprs | Content |
|---|---|---|---|
| 0 | Nav | Nav | Wordmark (the app icon + "hoprs"), links GitHub and Download, version `1.0.0` |
| 1 | Hero + app mockup | Hero | Headline, one sentence, **Download .deb** and **GitHub**; the glyph field behind; below the fold line a real screenshot of hoprs |
| 2 | Light on memory | Light and fast | The measured numbers printed as a tool's output: 84 ms to open, 33 MB idle, p95 0.58 ms at 100 000 rows, 0.99 × `scp` on 1 GiB SFTP; a link to `docs/perf/README.md` for the method |
| 3 | Agents built in | One connection, two tabs | Terminal and files over the same SSH connection; a real screenshot of a terminal tab next to a files tab |
| 4 | (product-specific) | Your FileZilla sites, kept | Import from `sitemanager.xml`, passwords in the system keyring, encrypted backup; shown as a diff (`- filezilla`, `+ hoprs`) of what changes |
| 5 | Open source, built on Zed | Open source | MIT OR Apache-2.0, built on iced, russh, suppaftp, alacritty_terminal; link to the repository |
| 6 | Ready when you are | Install | A well with tabs (`apt`, `checksum`, `from source`), copy buttons; the `.deb` link points to the release asset; "Ubuntu 24.04 or newer, amd64" |
| 7 | FAQ | FAQ | Five to seven questions: Windows/macOS? Where are passwords stored? Is plain FTP allowed? Does it import from FileZilla? Does it collect data (no)? How do I uninstall? How do I report a bug? |
| 8 | Footer | Footer | Wordmark, links (GitHub, Releases, Changelog, Security, License), license line |

Draft hero copy (the builder refines it within the Floor's limits): headline
"SSH, SFTP and FTP, in one quiet window."; sentence "A fast, native file client for Linux,
written in Rust."; actions "Download .deb" and "GitHub".

All copy is English. Every fact on the page must be traceable to the README, the CHANGELOG or
`docs/perf/README.md`.

## 5. The hero glyph field (approach B)

- A full-bleed canvas behind the hero text (`pointer-events: none`, `aria-hidden`).
- **Simulation:** a small stable-fluids solver in WebGL2 (splat, advect, divergence, Jacobi
  pressure, gradient subtract), as in the reference, at a low grid resolution (about 128
  columns). The cursor's motion injects velocity and "dye"; dye decays over about a second.
- **Rendering:** the dye field is sampled per character cell. A JetBrains Mono glyph atlas
  (rendered once to a texture at load) maps density to a glyph and a grey level from the
  `--glyph-*` ramp; dim cells draw smaller. The glyphs are hoprs' vocabulary in order: e.g.
  `ssh sftp ftps put get mkdir chmod 0644 drwxr-xr-x ~/ .toml known_hosts`.
- **At rest** (no cursor): a faint static field of dim glyphs, plus a slow ambient splat every
  few seconds so touch devices and idle visitors still see it move ("auto" mode, as Wu does on
  touch).
- **Budget:** pauses when the hero is off screen (IntersectionObserver) or the tab is hidden;
  caps device pixel ratio at 2; must keep the main thread free enough that scrolling stays at
  60 fps on an integrated GPU.
- **Fallbacks:** no WebGL2 or `prefers-reduced-motion: reduce` → a static `<pre>` glyph field
  (prerendered), no animation. Scripting off → the same static field; all content visible.

## 6. Build and hosting

- `site/`: Vite, React 19, TypeScript, Tailwind CSS v4 (CSS-first config, tokens in
  `:root`), `npm`. Fonts self-hosted (`@fontsource-variable/inter`,
  `@fontsource-variable/jetbrains-mono`), no third-party requests at runtime.
- **Prerender** the single route at build time to `dist/index.html` (e.g.
  `vite-react-ssg` or a 20-line `renderToString` script) so the content is in the HTML;
  React hydrates for the copy buttons and the canvas.
- The version, release asset URL and perf numbers come from one `site/src/facts.ts`, filled
  by hand per release (a test checks the version matches the workspace `Cargo.toml`).
- Screenshots: captured from the real app connected to the Docker test servers
  (`docker/compose.yml`), PNG → AVIF/WebP with explicit width and height; committed under
  `site/public/shots/`.
- `<head>`: title, description, canonical `https://hoprs.online/`, Open Graph and Twitter
  card with a 1200 x 630 image made from a screenshot, favicon from
  `docs/superpowers/specs/assets/2026-11-13-hoprs-icon.svg`.

**Hosting (already in place, 2026-11-14):** Cloudflare zone `hoprs.online` (Free), `A @ →
92.113.39.90` and `CNAME www → hoprs.online`, both proxied; SSL Full (strict), Always HTTPS,
TLS ≥ 1.2. Nameservers at Hostinger changed to `adaline` / `chuck.ns.cloudflare.com`.

**VPS (to set up, same pattern as `archviewer`):**

- User `hoprs-site`, files in `/var/www/hoprs/releases/<UTC timestamp>-<short sha>`, symlink
  `/var/www/hoprs/current`, keep the last 5 releases.
- `~hoprs-site/.ssh/authorized_keys`: one deploy key with
  `restrict,command="/usr/local/bin/hoprs-site-receive"`; the receive script reads a tarball
  on stdin, unpacks it into a new release folder, flips `current` atomically, prunes.
- KoreGo: domain `hoprs.online` (and `www.hoprs.online` redirecting to the apex) with a
  static route `/` → root `/var/www/hoprs/current`, SPA off, immutable prefix `/assets/`,
  TLS from a Cloudflare Origin certificate for `hoprs.online, *.hoprs.online`, Cloudflare-only
  on, security headers as on the author's other sites.
- `.github/workflows/site.yml`: on push to `main` touching `site/**` (and on
  `workflow_dispatch`): `npm ci`, lint, typecheck, test, build, then
  `tar -C dist -cz . | ssh hoprs-site@92.113.39.90` with the private key from a repository
  secret and the host key pinned in the workflow. Pull requests build and test but do not
  deploy.

## 7. Quality bar

- Lighthouse (mobile) ≥ 95 for performance, accessibility, best practices and SEO.
- Page weight without screenshots under 150 KB gzip JS; the glyph field adds no more than
  25 KB.
- Works at 360 px wide with no horizontal scroll; the hero headline, sentence and both actions
  are visible at 1280 x 800.
- Keyboard: every link and button reachable with visible focus; copy buttons announce
  "Copied" to screen readers.
- `sloplint --render` clean (Floor 0 FAIL).

## 8. Settled after the builder check

- **Download link:** the release asset name is versioned, so `facts.ts` holds the exact URL,
  `https://github.com/engviniciusbs/hoprs/releases/download/v1.0.0/hoprs_1.0.0-1_amd64.deb`,
  and the checksum file `SHA256SUMS` beside it (format: `sha256sum` output). The `apt` tab is
  `wget <url>` then `sudo apt install ./hoprs_1.0.0-1_amd64.deb`; the checksum tab is
  `wget <SHA256SUMS url>` then `sha256sum -c SHA256SUMS --ignore-missing`. (Controller's call.)
- **Screenshots:** none exist yet. The build captures them itself: run the release binary
  (under `xvfb-run` or on the author's display) with sites pointing at the Docker test
  servers, at 1440 x 900 and device scale 2. Until captured, the layout holds sized slots at
  16:10. (Controller's call.)
- **Hallmark `sloplint`** is a local gate (`~/.claude/skills/hallmark/scripts/sloplint.mjs`),
  run before each deploy by the builder, not in CI. (Controller's call.)
- **Glyph field:** hand-written WebGL2, no 3D or fluid library; the 25 KB gzip budget covers
  solver, shaders and renderer; the glyph atlas is drawn at runtime from the self-hosted
  font, and the prerendered `<pre>` fallback is HTML, not counted. (Controller's call.)
- **Numbers' conditions:** the "Light and fast" section carries one muted line, "Measured on a
  GitHub Actions runner, 2026-11-13", and links to `docs/perf/README.md`. (Controller's call.)
- **Footer links:** `CHANGELOG.md` exists; there is no `SECURITY.md`, so "Security" links to
  the README's Security section. (Controller's call.)
- **VPS and KoreGo:** the builder has root SSH to the VPS and does the server side (user,
  receive script modelled on `/usr/local/bin/archviewer-receive`, folders). The receive script
  is also committed under `site/deploy/` for review.
- **KoreGo domain and Origin certificate (author's answer):** the builder does it through the
  KoreGo API with a login the author provides at deploy time. The Origin certificate is
  created with the Cloudflare API from a CSR generated on the VPS, so its private key never
  leaves the server. The builder never edits `korego.db` directly.

## 9. Left to the builder

Exact copy within the limits above; the glyph vocabulary; the FAQ wording; which screenshots
(from the real app); the Open Graph image composition; the prerender tool.
