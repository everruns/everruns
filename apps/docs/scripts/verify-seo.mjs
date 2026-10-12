// Verifies the search metadata of every built page: exactly one canonical URL
// and og:url, both naming the page's own address; one <title>, unique across
// the site; a meta description of search-snippet length; an og:image that the
// build actually contains; one <h1>; and a noindex 404. A 2026-10-12 audit found
// 29 API tag pages all titled "Overview", missing social cards and descriptions
// past 200 characters; this keeps them from coming back.
// Requirements live in knowledge/ui/documentation.md.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { docsUrl } from "../site.config.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const distRoot = path.resolve(scriptDir, "../dist");
const site = docsUrl("/").replace(/\/$/, "");

// Search results show about 160 characters. API operation descriptions come
// from the OpenAPI spec and are trimmed by src/routeData.ts.
const MAX_DESCRIPTION = 170;
const MIN_DESCRIPTION = 50;

const errors = [];
const fail = (route, message) => errors.push(`${route}: ${message}`);

const decode = (value) =>
  value
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");

/** Content of every <meta {attr}="{name}" content="...">, attribute order free. */
function meta(html, attr, name) {
  const tags = html.match(new RegExp(`<meta(?:[^>"]|"[^"]*")*?${attr}="${name}"(?:[^>"]|"[^"]*")*>`, "g")) ?? [];
  return tags.map((tag) => decode(tag.match(/content="([^"]*)"/)?.[1] ?? ""));
}

function pages(directory, route = "/") {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) return pages(file, `${route}${entry.name}/`);
    return entry.name === "index.html" ? [{ route, file }] : [];
  });
}

const titles = new Map();
let checked = 0;
for (const { route, file } of pages(distRoot)) {
  const html = readFileSync(file, "utf8");
  if (html.includes('http-equiv="refresh"')) continue;
  checked++;

  const canonical = [...html.matchAll(/<link[^>]*rel="canonical"[^>]*href="([^"]*)"/g)].map((m) => m[1]);
  if (canonical.length !== 1 || canonical[0] !== `${site}${route}`) {
    fail(route, `canonical ${JSON.stringify(canonical)} should be ${site}${route}`);
  }
  const ogUrl = meta(html, "property", "og:url");
  if (ogUrl.length !== 1 || ogUrl[0] !== `${site}${route}`) fail(route, `og:url ${JSON.stringify(ogUrl)}`);
  if (/<meta[^>]*name="robots"[^>]*noindex/.test(html)) fail(route, "page is noindex");

  const title = [...html.matchAll(/<title>([^<]*)<\/title>/g)].map((m) => decode(m[1]));
  if (title.length !== 1) fail(route, `${title.length} <title> elements`);
  else titles.set(title[0], [...(titles.get(title[0]) ?? []), route]);

  const description = meta(html, "name", "description");
  if (description.length !== 1) fail(route, `${description.length} meta descriptions`);
  else if (description[0].length < MIN_DESCRIPTION || description[0].length > MAX_DESCRIPTION) {
    fail(route, `description is ${description[0].length} characters; keep it ${MIN_DESCRIPTION}-${MAX_DESCRIPTION}`);
  }

  const image = meta(html, "property", "og:image");
  if (image.length !== 1) fail(route, `${image.length} og:image tags`);
  else if (image[0].startsWith(`${site}/`) && !existsSync(path.join(distRoot, image[0].slice(site.length + 1)))) {
    fail(route, `og:image ${image[0]} is not in the build`);
  }

  const headings = html.match(/<h1[\s>]/g)?.length ?? 0;
  if (headings !== 1) fail(route, `${headings} <h1> elements`);
}

for (const [title, routes] of titles) {
  if (routes.length > 1) fail(routes.join(", "), `share the title "${title}"`);
}

const notFound = path.join(distRoot, "404.html");
if (!existsSync(notFound)) errors.push("missing from build output: 404.html");
else if (!/<meta[^>]*name="robots"[^>]*noindex/.test(readFileSync(notFound, "utf8"))) {
  errors.push("404.html: mark it noindex");
}

if (checked < 150) errors.push(`only ${checked} pages checked; the build looks incomplete`);
if (errors.length > 0) {
  console.error("Search metadata failed verification:");
  for (const error of errors.slice(0, 50)) console.error(`  - ${error}`);
  if (errors.length > 50) console.error(`  ... and ${errors.length - 50} more`);
  process.exit(1);
}
console.log(`Verified search metadata on ${checked} pages: canonicals, titles, descriptions, social cards.`);
