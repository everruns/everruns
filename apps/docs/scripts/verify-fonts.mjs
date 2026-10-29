// Keep font preloads aligned with the CSS assets: a different URL downloads twice.
import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import path from "node:path";
import { BASE } from "../site.config.mjs";

const root = new URL("../dist/", import.meta.url);
const css = readdirSync(new URL("_astro/", root))
  .filter((name) => name.endsWith(".css"))
  .map((name) => readFileSync(new URL(`_astro/${name}`, root), "utf8"))
  .join("\n");
let checked = 0;
function verify(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) verify(file);
    else if (entry.name.endsWith(".html")) {
      const html = readFileSync(file, "utf8");
      if (html.includes('http-equiv="refresh"')) continue;
      for (const family of ["geist", "geist-mono"]) {
        const tags = html.match(/<link\b[^>]*>/g) ?? [];
        const preload = tags.filter((tag) => tag.includes('rel="preload"') && tag.includes(`/${family}-latin-wght-normal.`));
        assert.equal(preload.length, 1, `${file}: one ${family} preload`);
        assert.match(preload[0], /as="font"/);
        assert.match(preload[0], /crossorigin/);
        const href = preload[0].match(/href="([^"]+)"/)[1];
        assert.ok(href.startsWith(`${BASE}/_astro/`), href);
        assert.ok(existsSync(new URL(href.slice(BASE.length + 1), root)), href);
        assert.ok(css.includes(path.basename(href)), `CSS must use ${href}`);
      }
      checked++;
    }
  }
}
verify(root.pathname);
console.log(`Verified font preloads on ${checked} pages`);
