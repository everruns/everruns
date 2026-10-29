import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import { docsUrl } from "../site.config.mjs";
import sitemapEnhance from "./sitemap-enhance.mjs";

test("sitemap advertises canonical pages while preserving ordinary history", async () => {
  const dir = mkdtempSync(join(tmpdir(), "docs-sitemap-"));
  const old = docsUrl("/changelog/2026-10-28-agent-cost-estimation/");
  const canonical = docsUrl("/changelog/2026-10-28-smart-agent-cost-projections/");
  const history = docsUrl("/changelog/another-historical-entry/");
  const lookalike = docsUrl("/changelog/2026-10-28-agent-cost-estimation-extra/");
  try {
    writeFileSync(join(dir, "sitemap-0.xml"),
      `<urlset>${[old, canonical, history, lookalike].map(url => `<url><loc>${url}</loc></url>`).join("")}</urlset>`);
    await sitemapEnhance().hooks["astro:build:done"]({ dir: pathToFileURL(`${dir}/`) });
    const sitemap = readFileSync(join(dir, "sitemap.xml"), "utf8");
    const locations = [...sitemap.matchAll(/<loc>(.*?)<\/loc>/g)].map(match => match[1]);
    assert.deepEqual(locations, [canonical, history, lookalike]);
    assert.match(sitemap, /<lastmod>\d{4}-\d{2}-\d{2}<\/lastmod>/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
