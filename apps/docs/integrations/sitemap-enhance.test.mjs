import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import sitemapEnhance from "./sitemap-enhance.mjs";

test("sitemap lists canonical pages, excluding the redirected announcement", async () => {
  const directory = mkdtempSync(join(tmpdir(), "docs-sitemap-"));
  const old = "https://everruns.com/docs/changelog/2026-10-28-agent-cost-estimation/";
  const successor = "https://everruns.com/docs/changelog/2026-10-28-smart-agent-cost-projections/";
  const gallery = "https://everruns.com/docs/features/generative-ui-gallery/";
  const preview = "https://everruns.com/docs/previews/generative-ui/";
  const other = "https://everruns.com/docs/features/ui/";
  try {
    writeFileSync(join(directory, "sitemap-0.xml"),
      `<urlset>${[old, successor, other, preview, gallery].map((url) => `<url><loc>${url}</loc></url>`).join("")}</urlset>`);
    const dates = { "changelog/2026-10-28-smart-agent-cost-projections": "2026-10-28" };
    await sitemapEnhance({ lastmod: () => (path) => dates[path] }).hooks["astro:build:done"]({
      dir: pathToFileURL(`${directory}/`),
    });
    const sitemap = readFileSync(join(directory, "sitemap.xml"), "utf8");
    assert.equal(sitemap.includes(old), false);
    assert.equal(sitemap.includes(preview), false);
    assert.equal(sitemap.includes(gallery), false);
    assert.ok(sitemap.includes(successor));
    assert.ok(sitemap.includes(other));
    // Only the page git could date carries <lastmod>, never the build date.
    assert.ok(sitemap.includes(`<loc>${successor}</loc><lastmod>2026-10-28</lastmod>`));
    assert.ok(sitemap.includes(`<url><loc>${other}</loc></url>`));
    assert.equal((sitemap.match(/<lastmod>/g) ?? []).length, 1);
    const index = readFileSync(join(directory, "sitemap-index.xml"), "utf8");
    assert.ok(index.includes("<lastmod>2026-10-28</lastmod>"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("no git dates: no <lastmod> anywhere", async () => {
  const directory = mkdtempSync(join(tmpdir(), "docs-sitemap-"));
  try {
    writeFileSync(join(directory, "sitemap-0.xml"),
      "<urlset><url><loc>https://everruns.com/docs/features/ui/</loc></url></urlset>");
    await sitemapEnhance({ lastmod: () => null }).hooks["astro:build:done"]({
      dir: pathToFileURL(`${directory}/`),
    });
    assert.equal(readFileSync(join(directory, "sitemap.xml"), "utf8").includes("<lastmod>"), false);
    assert.equal(readFileSync(join(directory, "sitemap-index.xml"), "utf8").includes("<lastmod>"), false);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
