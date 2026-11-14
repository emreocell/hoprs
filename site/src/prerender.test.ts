// @vitest-environment node
import { existsSync, readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { faq } from "./sections/Faq";
import { heroHeadline } from "./sections/Hero";

const indexPath = new URL("../dist/index.html", import.meta.url);

// Runs after `npm run build`. Without dist/ it is skipped, and the name says why.
it.skipIf(!existsSync(indexPath))(
  "content is in the HTML (skipped when dist/ is missing: run npm run build first)",
  () => {
    const html = readFileSync(indexPath, "utf8");
    expect(html).toContain(`<h1`);
    expect(html).toContain(heroHeadline);
    expect(html).toContain("sudo apt install ./hoprs_1.0.0-1_amd64.deb");
    for (const { q } of faq) expect(html).toContain(q);
    expect(html).toContain('<link rel="canonical" href="https://hoprs.online/">');
  },
);
