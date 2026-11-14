// Run after `vite build`: writes one page per language from dist/index.html (the template),
// with that language's head tags and the server-rendered App in #root, then sitemap.xml and llms.txt.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { render } from "../src/entry-server";
import { langs, pagePath } from "../src/i18n";
import { headHtml, llmsTxt, sitemapXml } from "../src/seo";

const dist = new URL("../dist/", import.meta.url);
const template = readFileSync(new URL("index.html", dist), "utf8");

function swap(html: string, marker: string | RegExp, value: string): string {
  if (!(typeof marker === "string" ? html.includes(marker) : marker.test(html))) {
    throw new Error(`prerender: ${marker} not found in dist/index.html`);
  }
  return html.replace(marker, () => value);
}

for (const lang of langs) {
  let html = template;
  html = swap(html, '<html lang="en">', `<html lang="${lang}">`);
  html = swap(html, /<!-- app-head:[^>]*-->/, headHtml(lang));
  html = swap(html, '<div id="root"></div>', `<div id="root">${render(lang)}</div>`);
  const dir = new URL(`.${pagePath[lang]}`, dist);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL("index.html", dir), html);
  console.log(`prerendered dist${pagePath[lang]}index.html (${lang})`);
}

// lastmod: the build date (UTC).
writeFileSync(new URL("sitemap.xml", dist), sitemapXml(new Date().toISOString().slice(0, 10)));
writeFileSync(new URL("llms.txt", dist), llmsTxt());
console.log("wrote dist/sitemap.xml and dist/llms.txt");
