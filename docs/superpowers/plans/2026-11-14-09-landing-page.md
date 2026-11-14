# Landing page (hoprs.online) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A static one-page site at https://hoprs.online that drives "Download .deb" and "GitHub", with a cursor-driven fluid hero drawn in monospace glyphs, deployed to the author's VPS behind Cloudflare.

**Architecture:** `site/` in this repo: Vite + React + TypeScript + Tailwind v4, prerendered to one `index.html` at build time and hydrated for the copy buttons and the canvas. The hero's glyph field is hand-written WebGL2 (stable-fluids solver + a glyph-atlas pass). A GitHub workflow builds on every `site/**` change and, on `main`, pipes the `dist/` tarball over SSH to a forced-command receive script on the VPS; KoreGo's Caddy serves `/var/www/hoprs/current`, Cloudflare-only.

**Tech Stack:** Node 22 LTS in CI (local is Node 26), npm, Vite 6, React 19, TypeScript 5, Tailwind CSS 4 (`@tailwindcss/vite`), Vitest + jsdom + Testing Library, `@fontsource-variable/inter`, `@fontsource-variable/jetbrains-mono`, raw WebGL2.

**Spec:** `docs/superpowers/specs/2026-11-14-landing-page-design.md`

## Global Constraints

- Every fact on the page traces to `README.md`, `CHANGELOG.md` or `docs/perf/README.md`; no invented metric, quote, logo or FileZilla number (spec §1 anti-goals).
- All copy in English. Two actions only: primary **Download .deb**, secondary **GitHub**.
- Palette from the app (spec §3): paper `oklch(13% 0 0)` / `16.5%` / `21%`; ink `97%`, ink-2 `84%`, muted `63%`; rule `25%`, rule-2 `34%`; accent `oklch(98% 0 0)`, accent-ink `oklch(14% 0 0)`, accent-wash `oklch(24% 0 0)`; glyph ramp `--glyph-0..7` grey, 30% → 97% lightness, chroma 0.
- Type: JetBrains Mono for heads, figures, commands, paths and glyphs; Inter for body text. Radius 4px on buttons and wells, square elsewhere; 1px rules; no shadows.
- Hallmark Floor (`~/.claude/skills/hallmark/SKILL.md` § The Floor) applies in full; the CSS source's first line is the Hallmark stamp; `node ~/.claude/skills/hallmark/scripts/sloplint.mjs <file> --render` must report 0 FAIL before deploy.
- No runtime third-party requests (fonts self-hosted, no analytics, no CDN).
- Glyph field: hand-written WebGL2, no library; ≤ 25 KB gzip for solver + shaders + renderer; paused off screen and when the tab is hidden; DPR capped at 2; `prefers-reduced-motion: reduce`, no WebGL2, or no script → the static prerendered `<pre>`.
- Download URLs are versioned (spec §8): `https://github.com/engviniciusbs/hoprs/releases/download/v1.0.0/hoprs_1.0.0-1_amd64.deb` and `.../v1.0.0/SHA256SUMS`.
- Quality bar (spec §7): Lighthouse mobile ≥ 95 in all four categories; JS ≤ 150 KB gzip excluding screenshots; no horizontal scroll at 360 px; hero headline, sentence and both actions visible at 1280 x 800.
- Repo conventions: commits in English ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; commit with `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit`; never change git config; work on branch `plan-9-landing-page`.
- Secrets (root password, KoreGo login, deploy private key, Origin certificate key) never enter the repository, a log line or a committed file.

## Review Focus

1. **Phone (360 px, touch, no hover):** the hero still moves (ambient splats), the install well's command scrolls inside the well, not the page. Test: Task 3 `layout.test.tsx` "install well never widens the page"; Task 5 sweep at 360 px.
2. **Copy button where the Clipboard API is refused** (permissions denied, insecure context): the command is selected so the visitor can press Ctrl+C, and the button says so. Test: Task 3 `InstallWell.test.tsx` "falls back to selecting the text".
3. **WebGL context lost** (GPU reset, many tabs): the canvas is removed and the static `<pre>` shows again, no exception loop. Test: Task 4 `GlyphField.test.tsx` "context loss shows the static field".
4. **Reduced motion / no WebGL2 / JS off:** the static field and all content render; no animation frame is requested. Test: Task 4 `GlyphField.test.tsx` "reduced motion never starts the loop"; Task 2 `prerender.test.ts` "content is in the HTML".
5. **Stale facts after a release:** the page advertising an old version. Test: Task 2 `facts.test.ts` "version matches the workspace Cargo.toml".

---

### Task 1: Scaffold `site/` with tokens, fonts and CI build

**Files:**
- Create: `site/package.json`, `site/package-lock.json`, `site/vite.config.ts`, `site/tsconfig.json`, `site/eslint.config.js`, `site/index.html`, `site/src/main.tsx`, `site/src/App.tsx`, `site/src/styles/tokens.css`, `site/src/styles/base.css`, `site/src/styles/app.css`, `site/.gitignore`
- Create: `.github/workflows/site.yml`
- Test: `site/src/App.test.tsx`

**Interfaces:**
- Produces: npm scripts `dev`, `build` (typecheck + vite build + prerender, wired in Task 2), `test` (`vitest run`), `lint` (`eslint .`), `typecheck` (`tsc --noEmit`); CSS tokens named exactly as in Global Constraints (`--color-paper`, `--color-paper-2`, `--color-paper-3`, `--color-ink`, `--color-ink-2`, `--color-muted`, `--color-rule`, `--color-rule-2`, `--color-accent`, `--color-accent-ink`, `--color-accent-wash`, `--color-focus`, `--glyph-0`…`--glyph-7`, `--font-display` = JetBrains Mono, `--font-body` = Inter, `--radius` = 4px) exposed to Tailwind via `@theme`.

- [ ] **Step 1:** `git switch -c plan-9-landing-page`. Generate Hallmark's starting files: `node ~/.claude/skills/hallmark/scripts/theme.mjs scaffold terminal` into `site/src/styles/`, then replace the Terminal palette and fonts in `tokens.css` with the app's values above (keep the shared scale). The stamp goes on line 1 of `app.css` with `theme: terminal (by request, palette and type from the hoprs app)`.
- [ ] **Step 2: Write the failing test** `App.test.tsx`: renders `<App />`; asserts a `main` landmark and a heading level 1 exist.
- [ ] **Step 3:** Run `npm --prefix site test` → FAIL (no App).
- [ ] **Step 4:** Minimal `App` with `<main><h1>hoprs</h1></main>`; Tailwind 4 via `@tailwindcss/vite`; fonts imported from the two `@fontsource-variable` packages in `main.tsx`; `html, body { overflow-x: clip; background: var(--color-paper); }`.
- [ ] **Step 5:** `npm --prefix site run lint && npm --prefix site run typecheck && npm --prefix site test && npm --prefix site run build` → all pass.
- [ ] **Step 6:** `.github/workflows/site.yml`: triggers `push` on `main` and `pull_request`, both with `paths: ['site/**', '.github/workflows/site.yml', 'Cargo.toml']`, plus `workflow_dispatch`; `permissions: contents: read`; job `build`: checkout (`persist-credentials: false`), `actions/setup-node` with Node 22 and npm cache on `site/package-lock.json`, `npm ci`, lint, typecheck, test, build, upload `site/dist` as an artifact. (Deploy job comes in Task 6.)
- [ ] **Step 7: Commit** `feat(site): scaffold the landing page with the app's tokens`.

### Task 2: Facts, page content and prerender

**Files:**
- Create: `site/src/facts.ts`, `site/src/facts.test.ts`, `site/src/sections/{Nav,Hero,Fast,Connection,Filezilla,OpenSource,Install,Faq,Footer}.tsx`, `site/src/entry-server.tsx`, `site/scripts/prerender.tsx`, `site/src/prerender.test.ts`
- Modify: `site/src/App.tsx`, `site/package.json` (build script), `site/index.html` (head)

**Interfaces:**
- Produces:
  ```ts
  export const facts: {
    version: "1.0.0"; debName: "hoprs_1.0.0-1_amd64.deb"; debUrl: string; sumsUrl: string;
    repoUrl: "https://github.com/engviniciusbs/hoprs"; releasesUrl: string; changelogUrl: string;
    securityUrl: string; perfDocUrl: string;
    perf: { startupMs: 84; idleMb: 33; rowsP95Ms: 0.58; rows: 100000; sftpVsScp: 0.99; measuredOn: "2026-11-13" };
  }
  ```
  `securityUrl` = `${repoUrl}#security`; `perfDocUrl` = `${repoUrl}/blob/main/docs/perf/README.md`; `changelogUrl` = `${repoUrl}/blob/main/CHANGELOG.md`.
  Hero exposes a slot `<div data-slot="glyph-field">` that Task 4 fills, and a screenshot slot `<figure data-slot="shot-main">` (16:10) that Task 5 fills; Connection has `data-slot="shot-split"`.
  Install section renders `<InstallWell />` (Task 3) — until then a placeholder `<pre>` with the apt commands.
  `prerender.tsx` exports nothing; run as `tsx scripts/prerender.tsx` after `vite build` and `vite build --ssr src/entry-server.tsx`, it writes `renderToString(<App />)` into `dist/index.html`'s `<div id="root">`. `main.tsx` uses `hydrateRoot`.

- [ ] **Step 1: Write the failing tests.** `facts.test.ts`: "version matches the workspace Cargo.toml" (reads `../Cargo.toml`, regex `^version = "(.+)"` under `[workspace.package]`, equals `facts.version`); "debUrl names the version" (`debUrl` ends with `/v${version}/hoprs_${version}-1_amd64.deb`). `prerender.test.ts` (runs after build, skipped with a clear message if `dist/` is missing): "content is in the HTML" — `dist/index.html` contains the `<h1>` text, the string `sudo apt install ./hoprs_1.0.0-1_amd64.deb`, every FAQ question, and `<link rel="canonical" href="https://hoprs.online/">`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Write `facts.ts` and the sections, in the order of spec §4. Copy rules: hero headline ≤ 7 words, sentence ≤ 12 words (draft: "SSH, SFTP and FTP, in one quiet window." / "A fast, native file client for Linux, written in Rust."); ≤ 80 words per screen; 450 visible words max. Fast section: four figures printed as a tool's output (Terminal theme Range "figures as a tool's output"), each with a `# comment` label, plus the muted line `Measured on a GitHub Actions runner, ${perf.measuredOn}.` and a link to `perfDocUrl`. Filezilla section: a unified diff (Terminal Range "comparison as a diff") with `--- a/filezilla` / `+++ b/hoprs`; every `+` line must be a CHANGELOG feature (import from sitemanager.xml, passwords in the system keyring, encrypted backup, terminal and files over one connection). FAQ: the seven questions of spec §4 as `<details>`; answers from README (Windows/macOS: "Not packaged yet; the code is portable."). Footer links: GitHub, Releases, Changelog, Security, License (`${repoUrl}/blob/main/LICENSE-MIT`).
- [ ] **Step 4:** `<head>`: title `hoprs — SSH, SFTP and FTP client for Linux`, meta description (the hero sentence), canonical, `og:*` and `twitter:card=summary_large_image` with `og:image` `https://hoprs.online/og.png` (file comes in Task 5), favicon `site/public/icon.svg` copied from `docs/superpowers/specs/assets/2026-11-13-hoprs-icon.svg`, `theme-color` `#0d0d0d`.
- [ ] **Step 5:** `npm --prefix site run build && npm --prefix site test` → PASS. Open `npm --prefix site run preview` and check the page reads top to bottom at 1280 and 360 px.
- [ ] **Step 6: Commit** `feat(site): page sections, facts and prerender`.

### Task 3: Install well (tabs + copy)

**Files:**
- Create: `site/src/components/InstallWell.tsx`, `site/src/components/InstallWell.test.tsx`, `site/src/layout.test.tsx`
- Modify: `site/src/sections/Install.tsx`, `site/src/sections/Hero.tsx` (the hero's primary action stays a plain link to `facts.debUrl`)

**Interfaces:**
- Consumes: `facts`.
- Produces: `export function InstallWell(props: { tabs: { id: string; label: string; lines: string[] }[] }): JSX.Element`. Tabs used: `apt` (`wget ${debUrl}`, `sudo apt install ./${debName}`), `checksum` (`wget ${sumsUrl}`, `sha256sum -c SHA256SUMS --ignore-missing`), `source` (`git clone ${repoUrl}.git`, `cd hoprs`, `cargo run --release`).

- [ ] **Step 1: Write the failing tests** (`InstallWell.test.tsx`, Testing Library + user-event):
  - "tabs follow the ARIA tabs pattern": `role="tablist"`, arrow keys move selection, `aria-selected` on the active tab, panel `aria-labelledby` its tab.
  - "copy writes all lines joined by newline": mock `navigator.clipboard.writeText` resolving; click Copy; called with `"wget …\nsudo apt install ./hoprs_1.0.0-1_amd64.deb"`; button's accessible name becomes "Copied" and a `role="status"` region announces "Copied"; after 1400 ms it returns to "Copy".
  - "falls back to selecting the text": `writeText` rejects (or `navigator.clipboard` undefined); after click, `window.getSelection().toString()` equals the panel's command text and the status says "Press Ctrl+C to copy".
  - Prompt glyph `$` is rendered in a separate `aria-hidden` span and is not part of the copied text.
  `layout.test.tsx`: "install well never widens the page": the well's `pre` has `overflow-x: auto` and `max-width: 100%` (class or computed style), and the section's grid tracks use `minmax(0, 1fr)`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement (Terminal theme Controls: `--color-paper-2` well, 1px `--color-rule` border, active tab underlined in `--color-accent`, 16px copy icon as inline SVG, state switches in `steps(1)`; eight states on the tab and button incl. `:focus-visible`).
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** `feat(site): install well with tabs and copy`.

### Task 4: Hero glyph field (WebGL2 fluid rendered as glyphs)

**Files:**
- Create: `site/src/glyph/vocabulary.ts`, `site/src/glyph/field.ts`, `site/src/glyph/field.test.ts`, `site/src/glyph/atlas.ts`, `site/src/glyph/fluid.ts` (solver), `site/src/glyph/shaders.ts`, `site/src/glyph/run.ts`, `site/src/glyph/GlyphField.tsx`, `site/src/glyph/GlyphField.test.tsx`
- Modify: `site/src/sections/Hero.tsx`

**Interfaces:**
- Produces:
  ```ts
  // vocabulary.ts
  export const VOCAB: string; // "ssh sftp ftps put get mkdir chmod 0644 drwxr-xr-x ~/ .toml known_hosts …" (≥ 200 chars, ASCII only)
  export function glyphAt(i: number): string; // VOCAB[i mod VOCAB.length], spaces skipped
  // field.ts
  export function levelFor(density: number): number; // clamp to [0,1], quantise to 0..8; 0 is blank; levelFor(0)=0, levelFor(1)=8
  export function staticField(cols: number, rows: number, seed: number): string[]; // deterministic rows for the <pre>
  // atlas.ts
  export function buildAtlas(chars: string, cell: { w: number; h: number }, font: string): HTMLCanvasElement;
  // run.ts
  export function startField(canvas: HTMLCanvasElement, opts: { cols: number; onLost: () => void }): () => void; // returns stop
  // GlyphField.tsx
  export function GlyphField(): JSX.Element; // renders the static <pre> always; mounts the canvas over it only when allowed
  ```

- [ ] **Step 1: Write the failing tests.** `field.test.ts`: `levelFor(-1)=0`, `levelFor(0)=0`, `levelFor(0.5)=4`, `levelFor(1)=8`, `levelFor(2)=8`; `staticField(130, 40, 7)` returns 40 strings of length 130, identical for the same seed, different for seed 8, at most 35% non-space cells (the field is faint at rest); `glyphAt(i)` never returns a space. `GlyphField.test.tsx` (jsdom, `getContext` mocked): "reduced motion never starts the loop" (`matchMedia('(prefers-reduced-motion: reduce)')` true → no canvas element, `requestAnimationFrame` never called); "no WebGL2 keeps the static field" (`getContext('webgl2')` returns null → no canvas, `<pre aria-hidden>` present); "context loss shows the static field" (dispatch `webglcontextlost` on the canvas → canvas removed, `<pre>` visible, no further rAF calls).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement `vocabulary.ts`, `field.ts`, `GlyphField.tsx` (mount gate: `matchMedia` reduced motion false, WebGL2 available with a float colour buffer extension, `IntersectionObserver` and `visibilitychange` pause/resume). Run the tests → PASS.
- [ ] **Step 4:** Implement the solver in `fluid.ts` + `shaders.ts`: velocity and dye ping-pong framebuffers (half-float), passes splat → divergence → Jacobi pressure (20 iterations) → gradient subtract → advect velocity and dye; dye fade ≈ 1 s; grid ≈ 128 columns, rows from the aspect ratio and the cell aspect (glyph cells ~1.85 times as tall as wide). The display pass draws one quad per frame: for each character cell it samples dye, maps it with the same quantisation as `levelFor`, picks the glyph `glyphAt(cellIndex)` from the atlas texture, colours it from the `--glyph-*` ramp (read from computed style at mount), and draws dim cells at 62% size. Pointer: `pointermove` on `window` (passive) adds velocity proportional to movement; ambient mode adds one soft splat every ~3 s when no pointer moved for 4 s (and always on `pointer: coarse`).
- [ ] **Step 5:** Verify by hand in `npm --prefix site run dev`: moving the mouse stirs bright glyphs that settle; hidden tab → 0% CPU in the browser task manager; `gzip -c` of the built glyph chunk ≤ 25 KB (`npm --prefix site run build` then `gzip -c dist/assets/<glyph chunk>.js | wc -c`; split the chunk with a dynamic `import('./run')` from `GlyphField`).
- [ ] **Step 6: Commit** `feat(site): hero glyph field over a WebGL2 fluid`.

### Task 5: Real screenshots, OG image and the Hallmark sweep

**Files:**
- Create: `site/scripts/capture/Dockerfile`, `site/scripts/capture/run.sh`, `site/scripts/capture/sites.toml`, `site/public/shots/{main,split}.{avif,webp,png}`, `site/public/og.png`
- Modify: `site/src/sections/Hero.tsx`, `site/src/sections/Connection.tsx`

**Interfaces:**
- Consumes: the slots from Task 2; the release `.deb` (`facts.debUrl`) or `target/release/hoprs`; the Docker test servers (`docker/compose.yml`, SSH user `alice` with `docker/fixtures/id_ed25519`, FTP user `frank`).

- [ ] **Step 1:** `run.sh` brings up `docker/compose.yml`, then runs an Ubuntu 24.04 container (`--network host`) with `xvfb`, `xdotool`, `imagemagick` and the `.deb` installed, `HOME` seeded with `sites.toml` (two sites: an SSH site "build server" on the test sshd with key auth from the fixture key, an FTPS site on the test vsftpd), `ICED_BACKEND=tiny-skia`, screen 1440 x 900; it launches `hoprs`, uses `xdotool` to open the SSH site as files (main shot: two panes with a populated remote folder and one transfer in the queue) and as terminal + files (split shot: `ls -la` output in the terminal tab), and grabs each window with `import -window root`. Coordinates are found by capturing and looking; keep them in `run.sh`. No password or keyring is used (key auth only).
- [ ] **Step 2:** Convert to AVIF and WebP (`<picture>` with explicit `width`/`height`, `loading="lazy"` for the split shot, `fetchpriority="high"` not set on the main shot since it sits below the fold line). Compose `og.png` 1200 x 630: the main shot cropped on `--color-paper` with the wordmark (ImageMagick in the same container).
- [ ] **Step 3:** Hallmark sweep: `node ~/.claude/skills/hallmark/scripts/sloplint.mjs site/src/styles/app.css --genre atmospheric --url http://localhost:4173/ --render` against `npm --prefix site run preview`. Fix every FAIL, fix or waive every REFLEX (waivers on the stamp lines), then the finish review per `gates.md` § 6.
- [ ] **Step 4:** Lighthouse mobile: `npx lighthouse http://localhost:4173/ --preset=perf --form-factor=mobile` and the full categories run; all four ≥ 95; JS total gzip ≤ 150 KB (`du` of `dist/assets/*.js` gzipped). Check 360 px has no horizontal scroll and 1280 x 800 shows headline, sentence and both actions.
- [ ] **Step 5: Commit** `feat(site): real screenshots, og image, Hallmark sweep`.

### Task 6: VPS, KoreGo and the deploy job

**Files:**
- Create: `site/deploy/hoprs-site-receive` (copy of the script installed on the VPS), `site/deploy/README.md` (what is on the server and how to roll back)
- Modify: `.github/workflows/site.yml` (deploy job)

**Interfaces:**
- Consumes: root SSH to `92.113.39.90` (password given by the author in the session; never written to a file in the repo); the KoreGo login (the author provides it when this task starts); Cloudflare zone `592e88212971ff5d975dd569167f2bc4` (`hoprs.online`).
- Produces: user `hoprs-site`; `/var/www/hoprs/{releases,current}`; `/usr/local/bin/hoprs-site-receive`; repository secrets `SITE_DEPLOY_KEY` (private key) and the pinned host key in the workflow; KoreGo domains `hoprs.online` and `www.hoprs.online`.

- [ ] **Step 1:** Receive script = `/usr/local/bin/archviewer-receive` with `BASE=/var/www/hoprs` and the messages in English; same argument check (commit SHA via `SSH_ORIGINAL_COMMAND`), refuses a release without `index.html`, atomic `current` swap, keeps 5. Commit it under `site/deploy/`.
- [ ] **Step 2:** On the VPS as root: `useradd --system --create-home --shell /bin/bash hoprs-site`; `/var/www/hoprs/releases` owned by it; install the script `0755 root:root`; generate a deploy key pair locally in the scratchpad (`ssh-keygen -t ed25519 -N '' -C hoprs-site-deploy`), put the public half in `~hoprs-site/.ssh/authorized_keys` as `restrict,command="/usr/local/bin/hoprs-site-receive" ssh-ed25519 …`; `gh secret set SITE_DEPLOY_KEY < key`; delete the local private key afterwards. Verify: `tar -C site/dist -c . | ssh -i key hoprs-site@92.113.39.90 $(git rev-parse HEAD)` prints `published: /var/www/hoprs/releases/…`; `ssh -i key hoprs-site@… id` is refused (forced command).
- [ ] **Step 3:** Origin certificate: on the VPS, `openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout /root/hoprs-origin.key -subj /CN=hoprs.online` (key stays `0600` on the VPS); create the certificate with the Cloudflare API `POST /certificates` (`request_type: origin-ecc`, hostnames `hoprs.online`, `*.hoprs.online`, validity 5475 days, the CSR). Through the KoreGo API from the VPS itself (`curl` to `127.0.0.1:8000`, logged in with the author's credentials, session cookie in a `0600` temp file deleted afterwards): upload the certificate and key (name "Cloudflare origin hoprs"); create domain `hoprs.online` with `tls_mode: custom`, that certificate, `cloudflare_only: true`, `force_https: true`, `www_redirect: www_to_apex`, compression on, the same `headers_json` as `archviewer.viniciusbsarmento.com`, and one route `/` → `static`, root `/var/www/hoprs/current`, `spa: false`, `immutable_prefix: /assets/`. Read KoreGo's request shapes from its UI's network calls or its `/api/domains` responses first; never write `korego.db`. Delete `/root/hoprs-origin.key` once KoreGo has stored it.
- [ ] **Step 4:** Deploy job in `site.yml`: `needs: build`, `if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'`, `environment: production`, `concurrency: site-deploy`; downloads the artifact, writes the key from `SITE_DEPLOY_KEY` to a `0600` file, `known_hosts` from a `SITE_HOST_KEY` line pinned in the workflow (`ssh-keyscan` once now, pasted), runs `tar -C dist -c . | ssh -i key hoprs-site@92.113.39.90 "$GITHUB_SHA"`, then `curl -fsS https://hoprs.online/ | grep -q '<h1'`.
- [ ] **Step 5:** Verify once the Cloudflare zone is active (`dig NS hoprs.online` shows the Cloudflare pair): `curl -sI https://hoprs.online/` → 200 with `cf-ray`; `curl -sI https://www.hoprs.online/` → 301 to the apex; `curl -sk --resolve hoprs.online:443:92.113.39.90 https://hoprs.online/` from outside Cloudflare → blocked (Cloudflare-only); `/assets/*` served with `Cache-Control: public, max-age=31536000, immutable`.
- [ ] **Step 6: Commit** `ci(site): deploy the landing page to the VPS`.

### Task 7: Docs

**Files:**
- Modify: `README.md` (website link under the title), `CLAUDE.md` (where things are: `site/`, the spec and this plan; status line), GitHub repo homepage (`gh repo edit --homepage https://hoprs.online`), `packaging/linux/io.github.engviniciusbs.hoprs.metainfo.xml` (`<url type="homepage">https://hoprs.online</url>`, which also clears the `url-homepage-missing` follow-up)

- [ ] **Step 1:** Make the edits; `appstreamcli validate --no-net packaging/linux/io.github.engviniciusbs.hoprs.metainfo.xml` no longer reports `url-homepage-missing`; `scripts/check-deb.sh` still passes in CI (the metainfo is in the package).
- [ ] **Step 2: Commit** `docs: point to hoprs.online`.
