// Head tags, JSON-LD, sitemap.xml and llms.txt, built at prerender time from the dictionaries and
// facts.ts, so they never drift from the page. Pure functions: scripts/prerender.tsx writes them.
import { downloads, systems } from "./downloads";
import { facts } from "./facts";
import { dicts, formatNumber, langs, otherLang, pagePath } from "./i18n";
import type { Lang } from "./i18n";

export const pageUrl = (lang: Lang) => `${facts.siteUrl}${pagePath[lang]}`;
const abs = (path: string) => `${facts.siteUrl}${path}`;
const ogImage = { url: abs("/og.png"), width: 1200, height: 630 };
const shots = ["/shots/main.png", "/shots/split.png"];

const escapeHtml = (s: string) =>
  s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

/** The four downloadable assets of the current release. */
export const downloadUrls = [facts.debUrl, facts.dmgUrl, facts.msiUrl, facts.zipUrl];

export function jsonLd(lang: Lang): object {
  const t = dicts[lang];
  const url = pageUrl(lang);
  const author = { "@type": "Person", name: facts.maintainer, url: facts.maintainerUrl, sameAs: [facts.maintainerUrl] };
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebSite",
        "@id": `${url}#website`,
        name: "hoprs",
        url,
        inLanguage: lang,
        description: t.meta.description,
      },
      {
        "@type": "SoftwareApplication",
        "@id": `${url}#app`,
        name: "hoprs",
        description: t.meta.description,
        url,
        applicationCategory: "DeveloperApplication",
        operatingSystem: "Linux, macOS, Windows",
        softwareVersion: facts.version,
        datePublished: facts.releaseDate,
        downloadUrl: downloadUrls,
        installUrl: `${url}#install`,
        releaseNotes: facts.changelogUrl,
        license: [facts.licenseUrl, facts.apacheLicenseUrl],
        isAccessibleForFree: true,
        offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
        image: ogImage.url,
        screenshot: [
          { "@type": "ImageObject", url: abs(shots[0]), caption: t.hero.shotAlt },
          { "@type": "ImageObject", url: abs(shots[1]), caption: t.connection.shotAlt },
        ],
        // The app's own interface is in English on every page.
        inLanguage: "en",
        author,
        publisher: author,
        sameAs: [facts.repoUrl],
      },
      {
        "@type": "FAQPage",
        "@id": `${url}#faq`,
        url,
        inLanguage: lang,
        mainEntity: t.faq.items.map(({ q, a }) => ({
          "@type": "Question",
          name: q,
          acceptedAnswer: { "@type": "Answer", text: a },
        })),
      },
    ],
  };
}

/** Everything that goes in <head> after the static tags of index.html. */
export function headHtml(lang: Lang): string {
  const t = dicts[lang];
  const other = otherLang(lang);
  const url = pageUrl(lang);
  const e = escapeHtml;
  // "<" inside a <script> data block could end it early; < is the same character to a JSON parser.
  const ld = JSON.stringify(jsonLd(lang)).replaceAll("<", "\\u003c");
  return [
    `<title>${e(t.meta.title)}</title>`,
    `<meta name="description" content="${e(t.meta.description)}" />`,
    `<link rel="canonical" href="${url}" />`,
    ...langs.map((l) => `<link rel="alternate" hreflang="${l}" href="${pageUrl(l)}" />`),
    `<link rel="alternate" hreflang="x-default" href="${pageUrl("en")}" />`,
    `<link rel="icon" href="/favicon.ico" sizes="32x32" />`,
    `<link rel="icon" type="image/svg+xml" href="/icon.svg" />`,
    `<link rel="apple-touch-icon" href="/apple-touch-icon.png" />`,
    `<meta property="og:type" content="website" />`,
    `<meta property="og:site_name" content="hoprs" />`,
    `<meta property="og:locale" content="${t.meta.ogLocale}" />`,
    `<meta property="og:locale:alternate" content="${dicts[other].meta.ogLocale}" />`,
    `<meta property="og:title" content="${e(t.meta.title)}" />`,
    `<meta property="og:description" content="${e(t.meta.description)}" />`,
    `<meta property="og:url" content="${url}" />`,
    `<meta property="og:image" content="${ogImage.url}" />`,
    `<meta property="og:image:width" content="${ogImage.width}" />`,
    `<meta property="og:image:height" content="${ogImage.height}" />`,
    `<meta property="og:image:alt" content="${e(t.meta.ogImageAlt)}" />`,
    `<meta name="twitter:card" content="summary_large_image" />`,
    `<meta name="twitter:title" content="${e(t.meta.title)}" />`,
    `<meta name="twitter:description" content="${e(t.meta.description)}" />`,
    `<meta name="twitter:image" content="${ogImage.url}" />`,
    `<meta name="twitter:image:alt" content="${e(t.meta.ogImageAlt)}" />`,
    `<script type="application/ld+json">${ld}</script>`,
  ].join("\n    ");
}

/** dist/sitemap.xml: both pages, each with its hreflang alternates. */
export function sitemapXml(lastmod: string): string {
  const alternates = [
    ...langs.map((l) => `    <xhtml:link rel="alternate" hreflang="${l}" href="${pageUrl(l)}"/>`),
    `    <xhtml:link rel="alternate" hreflang="x-default" href="${pageUrl("en")}"/>`,
  ].join("\n");
  const urls = langs
    .map((l) => `  <url>\n    <loc>${pageUrl(l)}</loc>\n    <lastmod>${lastmod}</lastmod>\n${alternates}\n  </url>`)
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${urls}
</urlset>
`;
}

/** dist/llms.txt (https://llmstxt.org): the page's facts in plain Markdown, English. */
export function llmsTxt(): string {
  const t = dicts.en;
  const { perf } = facts;
  const n = (v: number) => formatNumber("en", v).replaceAll(" ", " ");
  const leads: Record<string, string> = { linux: t.install.linuxLead, macos: t.install.macLead, windows: t.install.windowsLead };
  return `# hoprs

> ${t.meta.description}

${t.hero.headline} ${t.hero.sentence} Version ${facts.version}, released ${facts.releaseDate}. ${t.openSource.lead}

## Downloads (version ${facts.version})

${systems.map((os) => `- [${downloads[os].name} ${downloads[os].ext}](${downloads[os].href}): ${leads[os]}`).join("\n")}
- [Windows portable .zip](${facts.zipUrl}): unzip anywhere, no installer.
- [SHA256SUMS](${facts.sumsUrl}): checksums for every file above.
- Linux install: \`sudo apt install ./${facts.debName}\`. macOS and Windows builds are not notarized or signed with a paid certificate; the page explains how to open them.

## Features

- ${t.connection.lead}
- SSH with a terminal; files over SFTP, FTP and FTPS (explicit and implicit). Plain FTP is allowed and labelled ftp (unencrypted).
- Passwords: ${t.faq.items[2].a}
- Data collection: ${t.faq.items[5].a}
- Measured on a GitHub Actions runner (${perf.measuredOn}): ${n(perf.startupMs)} ms to the first frame, ${n(perf.idleMb)} MB memory at idle, ${n(perf.rowsP95Ms)} ms p95 view and layout for ${n(perf.rows)} rows, SFTP 1 GiB at 50 ms RTT at ${n(perf.sftpVsScp)} x scp. Method: ${facts.perfDocUrl}

## Compared with FileZilla

- Kept: ${t.filezilla.kept}.
${t.filezilla.added.map((line) => `- Added: ${line}.`).join("\n")}

## Questions

${t.faq.items.map(({ q, a }) => `- ${q} ${a}`).join("\n")}

## Licence

MIT OR Apache-2.0: ${facts.licenseUrl} and ${facts.apacheLicenseUrl}

## Links

- [Source code on GitHub](${facts.repoUrl})
- [Releases](${facts.releasesUrl})
- [Changelog](${facts.changelogUrl})
- [Security](${facts.securityUrl})
- [Página em português](${pageUrl("pt-BR")})
`;
}
