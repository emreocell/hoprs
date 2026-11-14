// Run after `vite build`: writes the server-rendered App into dist/index.html's #root.
import { readFileSync, writeFileSync } from "node:fs";
import { render } from "../src/entry-server";

const target = new URL("../dist/index.html", import.meta.url);
const html = readFileSync(target, "utf8");
const marker = '<div id="root"></div>';
if (!html.includes(marker)) throw new Error(`prerender: ${marker} not found in dist/index.html`);
writeFileSync(target, html.replace(marker, () => `<div id="root">${render()}</div>`));
console.log("prerendered dist/index.html");
