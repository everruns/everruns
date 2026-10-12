#!/usr/bin/env node
// Copies the everruns.com header and footer into the docs build.
//
// Decision: the docs share everruns.com with the landing site
// (everruns/landing), so they show its header and footer instead of a look of
// their own. Landing publishes the chrome at /site-chrome/ (rendered markup
// plus chrome.css and chrome.js); this script snapshots it into
// src/site-chrome/, which the Header and Footer overrides embed. The snapshot is
// committed so a docs build never depends on the landing site being up, and
// `--check` (run daily by docs-deploy.yml) reports when the snapshot has
// drifted from what everruns.com serves.
//
// Root-relative links are made absolute (https://everruns.com/...) so the
// chrome also works on preview hosts, where only /docs/ is served.
//
// Usage:
//   node scripts/sync-site-chrome.mjs                     # from everruns.com
//   node scripts/sync-site-chrome.mjs --from ../landing/dist/client
//   node scripts/sync-site-chrome.mjs --check             # exit 1 on drift

import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SITE = "https://everruns.com";
const OUT = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../src/site-chrome",
);

const args = process.argv.slice(2);
const from = args.includes("--from") ? args[args.indexOf("--from") + 1] : SITE;
const check = args.includes("--check");

async function load(file) {
  if (/^https?:/.test(from)) {
    const response = await fetch(`${from}/site-chrome/${file}`);
    if (!response.ok)
      throw new Error(`${from}/site-chrome/${file}: HTTP ${response.status}`);
    return response.text();
  }
  const local = file.endsWith("/") ? `${file}index.html` : file;
  return readFile(path.join(from, "site-chrome", local), "utf8");
}

function extract(html, tag, className) {
  const match = html.match(
    new RegExp(`<${tag} class="er-chrome ${className}"[\\s\\S]*</${tag}>`),
  );
  if (!match)
    throw new Error(
      `no <${tag} class="er-chrome ${className}"> in the published chrome`,
    );
  return absolutize(match[0]);
}

function absolutize(html) {
  return html.replace(/\b(href|src)="\/(?!\/)([^"]*)"/g, `$1="${SITE}/$2"`);
}

const files = {
  "header.html": extract(await load("header/"), "header", "er-header"),
  "footer.html": extract(await load("footer/"), "footer", "er-footer"),
  "chrome.css": await load("chrome.css"),
  "chrome.js": await load("chrome.js"),
};

let drifted = [];
await mkdir(OUT, { recursive: true });
for (const [name, content] of Object.entries(files)) {
  const target = path.join(OUT, name);
  const current = await readFile(target, "utf8").catch(() => null);
  const next = content.trimEnd() + "\n";
  if (current === next) continue;
  if (check) drifted.push(name);
  else await writeFile(target, next);
}

if (check && drifted.length) {
  console.error(
    `Site chrome drifted from ${from}: ${drifted.join(", ")}.\n` +
      "Run `pnpm run sync-chrome` in apps/docs and commit src/site-chrome/.",
  );
  process.exit(1);
}
console.log(
  check ? `Site chrome matches ${from}` : `Site chrome synced from ${from}`,
);
