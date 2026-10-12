// Checks the published docs end to end, against the live site or a preview:
//
//   node scripts/check-live.mjs [--site https://everruns.com/docs] [--old https://docs.everruns.com]
//
// A preview (`--site https://<version>-everruns-docs.everruns.workers.dev/docs`)
// still lists canonical everruns.com/docs URLs in its sitemap and llms.txt;
// those are fetched from the preview instead.
//
// 1. Every page in the published sitemap answers 200 with HTML, and its
//    Markdown twin (`<page>index.md`) answers 200.
// 2. Agent-facing files resolve: llms.txt and its sets, the OpenAPI schema,
//    the agent schema, the changelog feed, the search index.
// 3. Content negotiation: a sample of pages returns Markdown for
//    `Accept: text/markdown` and HTML for a browser.
// 4. With --old: every page on the former docs.everruns.com host answers one
//    301 straight to the same path under the new site (query kept), and the
//    target answers 200. This is what keeps inbound links, published crate
//    docs and search rankings working, so it stays a scheduled check.
//
// Decision: page URLs come from the new sitemap, so the check follows the docs
// as they grow instead of a frozen list.
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
};
const site = option("--site", "https://everruns.com/docs").replace(/\/+$/, "");
const old = option("--old", "")?.replace(/\/+$/, "");
const concurrency = Number(option("--concurrency", "16"));
const CANONICAL = "https://everruns.com/docs";
const local = (url) => (url.startsWith(CANONICAL) ? `${site}${url.slice(CANONICAL.length)}` : url);

const failures = [];
const fail = (message) => failures.push(message);

async function request(url, init = {}) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await fetch(url, { redirect: "manual", ...init });
    } catch (cause) {
      if (attempt >= 3) throw cause;
      await new Promise((resolve) => setTimeout(resolve, 500 * attempt));
    }
  }
}

async function pool(items, worker) {
  let next = 0;
  await Promise.all(
    Array.from({ length: concurrency }, async () => {
      while (next < items.length) await worker(items[next++]);
    }),
  );
}

async function expectOk(url, contentType, init) {
  const response = await request(url, init);
  if (response.status !== 200) {
    fail(`${url}: ${response.status}, expected 200`);
    return undefined;
  }
  const type = response.headers.get("content-type") ?? "";
  if (contentType && !type.includes(contentType)) fail(`${url}: content-type ${type}, expected ${contentType}`);
  return response;
}

// Sitemap: the index points at the page list, the page list at the pages.
const sitemapIndex = await (await expectOk(`${site}/sitemap-index.xml`, "xml"))?.text();
const sitemapUrl = local(sitemapIndex?.match(/<loc>([^<]+)<\/loc>/)?.[1] ?? "");
if (!sitemapUrl.startsWith("http")) throw new Error(`${site}/sitemap-index.xml lists no sitemap`);
const sitemap = await (await expectOk(sitemapUrl, "xml"))?.text();
const pages = [...(sitemap ?? "").matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => local(match[1]));
if (pages.length < 150) fail(`${sitemapUrl} lists ${pages.length} pages, expected at least 150`);
for (const page of pages) {
  if (!page.startsWith(`${site}/`)) fail(`sitemap lists ${page}, outside ${site}/`);
}

await pool(pages, async (page) => {
  await expectOk(page, "text/html");
  await expectOk(`${page}index.md`, "text/markdown");
});

for (const [file, type] of [
  ["/llms.txt", "text/plain"],
  ["/llms-full.txt", "text/plain"],
  ["/llms-small.txt", "text/plain"],
  ["/api/openapi.json", "application/json"],
  ["/schemas/agent/v1.json", "application/json"],
  ["/changelog/rss.xml", "xml"],
  ["/pagefind/pagefind.js", "javascript"],
]) {
  await expectOk(`${site}${file}`, type);
}
const llms = await (await request(`${site}/llms.txt`)).text();
const sets = [...llms.matchAll(/\]\((https:[^)]+\.txt)\)/g)].map((match) => local(match[1]));
if (sets.length < 5) fail(`${site}/llms.txt links ${sets.length} documentation sets, expected at least 5`);
await pool(sets, (url) => expectOk(url, "text/plain"));

for (const page of pages.slice(0, 20)) {
  await expectOk(page, "text/markdown", { headers: { accept: "text/markdown" } });
  await expectOk(page, "text/html", { headers: { accept: "text/html", "user-agent": "Mozilla/5.0" } });
}

if (old) {
  const paths = pages.map((page) => page.slice(site.length));
  const extra = ["/llms.txt", "/api/openapi.json", "/schemas/agent/v1.json", "/framework/index.md"];
  await pool([...paths, ...extra, "/framework/quickstart/?ref=check"], async (path) => {
    const from = `${old}${path}`;
    const response = await request(from);
    const location = response.headers.get("location");
    if (response.status !== 301) return fail(`${from}: ${response.status}, expected 301`);
    if (location !== `${site}${path}`) return fail(`${from}: redirects to ${location}, expected ${site}${path}`);
    await expectOk(location);
  });
}

const checked = `${pages.length} pages${old ? ` and their redirects from ${old}` : ""}`;
if (failures.length) {
  console.error(`${failures.length} problem(s) checking ${checked} on ${site}:`);
  for (const message of failures.slice(0, 100)) console.error(`- ${message}`);
  process.exit(1);
}
console.log(`OK: ${checked} on ${site}.`);
