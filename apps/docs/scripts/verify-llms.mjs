// Verifies the agent-readable outputs in the build: per-page Markdown,
// /llms.txt, and the text sets it advertises. The docs build happily emits an
// llms.txt with no index, an "abridged" set the same size as the complete one,
// or pages whose Source URL points at a route that does not exist — all of which
// shipped unnoticed before this check existed. Requirements live in
// knowledge/ui/documentation.md.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const distRoot = path.resolve(scriptDir, "../dist");
const site = "https://docs.everruns.com";

/** Minimum share of docs pages expected in the complete text. */
const MIN_PAGES = 150;
/** The abridged set must be meaningfully smaller, not just whitespace-collapsed. */
const MAX_SMALL_SHARE = 0.75;

const errors = [];
const check = (condition, message) => {
  if (!condition) errors.push(message);
};

const read = (name) => {
  const file = path.join(distRoot, name);
  if (!existsSync(file)) {
    errors.push(`missing from build output: ${name}`);
    return null;
  }
  return readFileSync(file, "utf8");
};

const index = read("llms.txt");
const full = read("llms-full.txt");
const small = read("llms-small.txt");
const sitemap = read("sitemap.xml");

let pageMarkdownCount = 0;
if (sitemap) {
  const pageUrls = [...sitemap.matchAll(/<loc>(https:\/\/[^<]+)<\/loc>/g)]
    .map((match) => match[1])
    .filter((url) => !new URL(url).pathname.startsWith("/api/"));
  check(pageUrls.length > 0, "sitemap.xml contains no page URLs");

  for (const url of pageUrls) {
    if (!url.startsWith(`${site}/`)) {
      errors.push(`sitemap URL is not on ${site}: ${url}`);
      continue;
    }

    const route = decodeURIComponent(new URL(url).pathname).replace(/^\/+|\/+$/g, "");
    const relative = path.join(route, "index.md");
    const markdown = read(relative);
    if (markdown === null) continue;

    pageMarkdownCount += 1;
    check(markdown.length > 0, `per-page Markdown is empty: ${relative}`);
    check(markdown.startsWith("# "), `per-page Markdown has no page title: ${relative}`);
    check(
      markdown.includes(`Source: <${url}>`),
      `per-page Markdown has no canonical Source URL: ${relative}`
    );
    check(
      !markdown.includes("[Section titled "),
      `per-page Markdown contains a heading anchor artifact: ${relative}`
    );
    const relativeLinks = [...markdown.matchAll(/\]\((\/[^/)][^)]*)\)/g)];
    check(
      relativeLinks.length === 0,
      `per-page Markdown contains a root-relative link: ${relative}`
    );
  }
}

// llms.txt must be an index, not just a pointer to two dumps: the three ways to
// run Everruns, one link per documentation set, and the machine-readable
// surfaces that the sets deliberately leave out.
if (index) {
  for (const marker of ["Framework", "Self-hosted platform", "Hosted Everruns"]) {
    check(index.includes(marker), `llms.txt does not describe the ${marker} target`);
  }
  const setLinks = [...index.matchAll(/\]\((https:\/\/[^)]*\/_llms-txt\/[^)]+\.txt)\)/g)].map(
    (match) => match[1]
  );
  check(
    setLinks.length >= 5,
    `llms.txt advertises ${setLinks.length} documentation set(s), expected at least 5`
  );
  check(index.includes("## Optional"), "llms.txt has no Optional section");
  check(
    index.includes(`${site}/api/openapi.json`),
    "llms.txt does not link the OpenAPI schema"
  );

  for (const link of setLinks) {
    const relative = link.slice(site.length + 1);
    const file = path.join(distRoot, relative);
    if (!existsSync(file)) {
      errors.push(`documentation set advertised in llms.txt but not built: ${relative}`);
      continue;
    }
    const contents = readFileSync(file, "utf8");
    check(
      contents.includes("Source: <"),
      `documentation set has no pages: ${relative}`
    );
  }
}

// The OpenAPI schema is excluded from the text sets on the grounds that it is
// served as a schema, so it has to be there.
const schemaPath = path.join(distRoot, "api/openapi.json");
if (!existsSync(schemaPath)) {
  errors.push("api/openapi.json missing from build output");
} else {
  try {
    JSON.parse(readFileSync(schemaPath, "utf8"));
  } catch (cause) {
    errors.push(`api/openapi.json is not valid JSON: ${cause.message}`);
  }
}

if (full) {
  const sources = [...full.matchAll(/^Source: <([^>]+)>$/gm)].map((match) => match[1]);
  check(
    sources.length >= MIN_PAGES,
    `llms-full.txt carries ${sources.length} page(s), expected at least ${MIN_PAGES}`
  );

  // A Source URL that resolves nowhere is worse than no URL at all.
  for (const url of sources) {
    check(url.startsWith(`${site}/`), `Source URL is not on ${site}: ${url}`);
    const route = url.slice(site.length).replace(/^\/+/, "");
    const page = path.join(distRoot, route, "index.html");
    check(existsSync(page), `Source URL has no built page: ${url}`);
  }

  // Starlight's heading anchor links convert to screen-reader noise.
  const anchorNoise = (full.match(/^\[Section titled /gm) ?? []).length;
  check(anchorNoise === 0, `llms-full.txt contains ${anchorNoise} heading anchor artifact(s)`);
  check(
    !full.includes("sl-anchor-link"),
    "llms-full.txt contains raw Starlight anchor markup"
  );
  check(full.includes("\n---\n"), "llms-full.txt does not separate pages");

  // A root-relative link resolves against whoever is serving the reader, not
  // against the docs site.
  const relativeLinks = [...full.matchAll(/\]\((\/[^/)][^)]*)\)/g)].map((match) => match[1]);
  check(
    relativeLinks.length === 0,
    `llms-full.txt contains ${relativeLinks.length} root-relative link(s), ` +
      `first: ${relativeLinks[0]}`
  );
}

if (full && small) {
  const share = small.length / full.length;
  check(
    share <= MAX_SMALL_SHARE,
    `llms-small.txt is ${Math.round(share * 100)}% the size of llms-full.txt, ` +
      `expected at most ${Math.round(MAX_SMALL_SHARE * 100)}% — check excludeSmall`
  );
}

if (errors.length > 0) {
  console.error("Agent-readable docs output failed verification:");
  for (const error of errors) console.error(`  - ${error}`);
  process.exit(1);
}

const setDir = path.join(distRoot, "_llms-txt");
const sets = existsSync(setDir) ? readdirSync(setDir).filter((f) => f.endsWith(".txt")) : [];
const kb = (text) => `${Math.round(text.length / 1024)} KB`;
console.log(
  `Verified agent-readable docs output: ${pageMarkdownCount} per-page Markdown file(s), ` +
    `llms.txt index, ${sets.length} documentation set(s), ` +
    `llms-full.txt (${kb(full)}), llms-small.txt (${kb(small)}), ` +
    `api/openapi.json (${Math.round(statSync(schemaPath).size / 1024)} KB).`
);
