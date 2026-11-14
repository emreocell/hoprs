import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";

const file = resolve(process.cwd(), "dist/sitemap.xml");
const XHTML = "http://www.w3.org/1999/xhtml";

// Runs after `npm run build` (jsdom's DOMParser does the XML parsing). Skipped without dist/.
it.skipIf(!existsSync(file))("dist/sitemap.xml parses and lists both pages with hreflang alternates", () => {
  const doc = new DOMParser().parseFromString(readFileSync(file, "utf8"), "application/xml");
  expect(doc.getElementsByTagName("parsererror").length).toBe(0);
  expect(doc.documentElement.localName).toBe("urlset");
  const urls = [...doc.getElementsByTagName("url")];
  expect(urls.map((u) => u.getElementsByTagName("loc")[0]?.textContent)).toEqual([
    "https://hoprs.online/",
    "https://hoprs.online/pt-br/",
  ]);
  for (const u of urls) {
    expect(u.getElementsByTagName("lastmod")[0]?.textContent).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    const links = [...u.getElementsByTagNameNS(XHTML, "link")].map((l) => [l.getAttribute("hreflang"), l.getAttribute("href")]);
    expect(links).toEqual([
      ["en", "https://hoprs.online/"],
      ["pt-BR", "https://hoprs.online/pt-br/"],
      ["x-default", "https://hoprs.online/"],
    ]);
  }
});
