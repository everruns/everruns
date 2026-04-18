// Post-processes Starlight's sitemap to produce /sitemap.xml with <lastmod>.
// Uses build date (not git history) — works on Cloudflare Pages shallow clones.
import { readFileSync, writeFileSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export default function sitemapEnhance() {
  return {
    name: "sitemap-enhance",
    hooks: {
      "astro:build:done": async ({ dir }) => {
        const outDir = fileURLToPath(dir);
        const sitemapSrc = join(outDir, "sitemap-0.xml");

        let content;
        try {
          content = readFileSync(sitemapSrc, "utf-8");
        } catch {
          console.warn(
            "[sitemap-enhance] sitemap-0.xml not found, skipping"
          );
          return;
        }

        const lastmod = new Date().toISOString().split("T")[0];

        // Add <lastmod> to each <url> entry
        const enhanced = content.replace(
          /<url><loc>(.*?)<\/loc><\/url>/g,
          (_match, url) =>
            `<url><loc>${url}</loc><lastmod>${lastmod}</lastmod></url>`
        );

        const sitemapDest = join(outDir, "sitemap.xml");
        const sitemapIndexDest = join(outDir, "sitemap-index.xml");
        writeFileSync(sitemapDest, enhanced);
        writeFileSync(
          sitemapIndexDest,
          [
            '<?xml version="1.0" encoding="UTF-8"?>',
            '<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">',
            "  <sitemap>",
            "    <loc>https://docs.everruns.com/sitemap.xml</loc>",
            `    <lastmod>${lastmod}</lastmod>`,
            "  </sitemap>",
            "</sitemapindex>",
            "",
          ].join("\n")
        );

        // Remove Starlight's intermediate sitemap files
        unlinkSync(sitemapSrc);

        const entryCount = (enhanced.match(/<url>/g) || []).length;
        console.log(
          `[sitemap-enhance] sitemap.xml and sitemap-index.xml written with ${entryCount} entries (lastmod: ${lastmod})`
        );
      },
    },
  };
}
