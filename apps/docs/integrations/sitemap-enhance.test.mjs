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
    await sitemapEnhance().hooks["astro:build:done"]({ dir: pathToFileURL(`${directory}/`) });
    const sitemap = readFileSync(join(directory, "sitemap.xml"), "utf8");
    assert.equal(sitemap.includes(old), false);
    assert.equal(sitemap.includes(preview), false);
    assert.equal(sitemap.includes(gallery), false);
    assert.ok(sitemap.includes(successor));
    assert.ok(sitemap.includes(other));
    assert.equal((sitemap.match(/<lastmod>/g) ?? []).length, 2);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
