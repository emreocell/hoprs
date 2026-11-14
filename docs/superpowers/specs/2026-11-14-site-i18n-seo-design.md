# hoprs.online: OS downloads in the hero, OS logos, PT-BR, SEO — design

Status: approved by the author's request (2026-11-14): "na hero tem um button para download, mas é
apenas para Linux; adicione os logos dos OS nos buttons de download; adicione suporte ao PT-BR no
site; deixe o SEO o melhor possível para o site ser recomendado pelo google e pelas IAs".
Code: `site/` (Vite + React + TS + Tailwind 4, prerendered by `site/scripts/prerender.tsx`).
Design rules: the Hallmark stamp at the top of the site's CSS (tokens only, no tracked caps labels,
no emoji icons, ~80 words a screen, hero = headline + one sentence + actions). `sloplint --render`
must report no FAIL beyond the two recorded as answered (gates 40 and 58).

## 1. Hero: a download for every system

Today the hero has one primary button that follows `useDetectedOs()` (prerender: Linux). Change:
the hero shows **three** download buttons, Linux (`.deb`), macOS (`.dmg`), Windows (`.msi`); the
detected system's button is the primary style and comes first, the other two use the existing
secondary button style. Prerender order: Linux first (as today), client reorders after hydration.
The existing secondary hero action (if any, e.g. "View on GitHub") stays.

## 2. OS logos on download buttons

Monochrome inline SVG logos (Tux for Linux, the Apple mark for macOS, the Windows four-pane mark)
drawn in `currentColor`, ~1em, `aria-hidden="true"` (the button text already names the system), in
one small component (`site/src/components/OsLogo.tsx`). Use simple-icons paths (CC0) for Apple and
Windows and a simple Tux silhouette (simple-icons "linux", CC0); note the source in a comment.
Used on the three hero buttons and on the three Install tabs.

## 3. Portuguese (pt-BR)

- English stays at `/` (default, `x-default`); Portuguese at `/pt-br/` (prerendered to
  `dist/pt-br/index.html`). Asset URLs stay absolute (`/assets/…`).
- All visible copy, `<title>`, meta description, OG/Twitter text, image alt texts and JSON-LD are
  translated. Copy lives in one dictionary per language (`site/src/i18n/en.ts`, `pt-br.ts`, typed so a
  missing key fails typecheck); components read the language from a React context set by the
  entry (`App lang="en" | "pt-BR"`). Facts (version, URLs, numbers) are shared, never duplicated.
- Number formatting per locale (pt-BR: `1.000` grouping, comma decimals) through `Intl`.
- A language switch in the nav: a plain link "Português" on the English page and "English" on the
  Portuguese page, `hreflang` + `lang` attributes on the link. No automatic redirect by
  Accept-Language or `navigator.language` (bad for indexing).
- `<html lang>` per page: `en` / `pt-BR`.
- Translate naturally for Brazilian developers (not word for word); keep product terms in English
  where Brazilians use them (SSH, SFTP, FTP, terminal, workspace, upload). The Gatekeeper/SmartScreen
  steps use the Portuguese UI names macOS and Windows show in pt-BR ("Ajustes do Sistema →
  Privacidade e Segurança → Abrir Mesmo Assim"; "O Windows protegeu o computador" → "Mais
  informações" → "Executar assim mesmo").
- The deploy is unchanged: `dist/` (now with `pt-br/`) goes to the VPS; the static route already
  serves `/pt-br/` → `pt-br/index.html` (check `site/deploy/README.md`; if the route's `spa: false`
  does not serve directory indexes, make links point to `/pt-br/` and confirm Caddy's file_server
  index behaviour in the README).

## 4. SEO (from the audit of 2026-11-14)

For each page (en and pt-BR):
- `<link rel="canonical">` self-referencing; `<link rel="alternate" hreflang="en" href="https://hoprs.online/">`,
  `hreflang="pt-BR"` → `https://hoprs.online/pt-br/`, `hreflang="x-default"` → `/`.
- Meta description rewritten (≈140–160 chars) naming what it is and why: free, open source,
  SSH/SFTP/FTP/FTPS + terminal, FileZilla sites import, Linux/macOS/Windows. Title keeps
  "hoprs: SSH, SFTP and FTP client for Linux, macOS and Windows" (pt-BR: "hoprs: cliente SSH, SFTP e
  FTP para Linux, macOS e Windows").
- Open Graph: `og:locale` (`en_US` / `pt_BR`) and `og:locale:alternate`, `og:image:width`,
  `og:image:height`, `og:image:alt`; `twitter:title`, `twitter:description`, `twitter:image`.
- JSON-LD (`<script type="application/ld+json">`, one `@graph`):
  - `SoftwareApplication`: name, description, `applicationCategory: "DeveloperApplication"`,
    `operatingSystem: "Linux, macOS, Windows"`, `softwareVersion` (facts.version), `offers`
    (price 0, currency USD), `downloadUrl` (the four assets), `license` (both licences' URLs),
    `url`, `image`/`screenshot` (the shots), `author`/`publisher` (Person or Organization with the
    GitHub repo URL as `sameAs`), `codeRepository`-style `sameAs` to the GitHub repo,
    `inLanguage`, `isAccessibleForFree: true`. No invented ratings or reviews (never
    `aggregateRating`).
  - `FAQPage` built from the same FAQ items the page shows (same language).
  - `WebSite` with `name`, `url`, `inLanguage`.
  Generated from the dictionaries and facts at prerender time, so it never drifts from the page.
- `dist/sitemap.xml` (generated at build): both URLs, `lastmod` = build date, `xhtml:link`
  alternates for hreflang. `robots.txt` gains `Sitemap: https://hoprs.online/sitemap.xml`.
- `dist/llms.txt` (generated at build, per https://llmstxt.org): H1 "hoprs", a blockquote summary,
  then short sections — what it is, platforms and downloads (current version links), key features,
  how it compares with FileZilla (facts only, from the page), licence, links (GitHub, changelog,
  security, pt-BR page). Plain facts only, nothing the page does not say.
- `favicon.ico` (32/16 px from the existing icon) and `apple-touch-icon.png` (180 px), linked.
- Footer: a short maintainer/author line linking the GitHub profile/repo and "Updated <release
  date>" (date of the current release, kept in facts).
- No new tracking, no analytics, CSP stays `'self'`-only (JSON-LD is data, not executed).

## Tests

- Prerender test: both `dist/index.html` and `dist/pt-br/index.html` exist with the right `lang`,
  title, canonical, the three hreflang links, valid JSON-LD (parse it; check `@type`s, version,
  four downloadUrls, FAQ count = rendered FAQ items), OG locale tags.
- `sitemap.xml` parses and lists both URLs with alternates; `robots.txt` has the Sitemap line;
  `llms.txt` starts with `# hoprs` and contains the four download URLs.
- Dictionary parity: every key exists in both languages (typecheck) and no pt-BR value equals the
  English one except allowed product terms (a small allowlist).
- Hero renders three download links with the detected OS first; OsLogo renders an `aria-hidden` SVG.
- Existing tests keep passing; Lighthouse (mobile) stays ≥ 95 in every category on both pages.

## Left to the builder

Exact Portuguese copy (natural pt-BR), component structure, how the prerender script loops over
languages, icon generation tool for favicon.ico/apple-touch-icon.
