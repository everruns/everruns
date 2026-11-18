// Post-processes Starlight's sitemap to produce /sitemap.xml with <lastmod>:
// the date each page's content last changed in git (git-lastmod.mjs), never
// the build date. A page git cannot date gets no <lastmod>.
import { readFileSync, writeFileSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { docsUrl } from "../site.config.mjs";
import { changelogRedirect, galleryRedirect } from "../redirects.mjs";
import { docsLastmod } from "./git-lastmod.mjs";

const REPO_ROOT = fileURLToPath(new URL("../../../", import.meta.url));
const CONTENT_DIR = join(REPO_ROOT, "docs");

// `lastmod` maps a page path relative to the docs base ("framework/quickstart")
// to its date; tests pass their own.
export default function sitemapEnhance({
  lastmod = () => docsLastmod({ repoRoot: REPO_ROOT, contentDir: CONTENT_DIR }),
} = {}) {
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

        const dateOf = lastmod() ?? (() => undefined);
        const basePath = new URL(docsUrl("/")).pathname;
        let newest;
        let dated = 0;

        // Redirect sources are durable inbound links, not canonical sitemap pages.
        const enhanced = content.replace(
          /<url><loc>(.*?)<\/loc><\/url>/g,
          (_match, url) => {
            const parsed = new URL(url);
            if (changelogRedirect(parsed) || galleryRedirect(parsed) || parsed.pathname === "/docs/previews/generative-ui/") {
              return "";
            }
            const pagePath = parsed.pathname.slice(basePath.length).replace(/\/$/, "");
            const date = dateOf(pagePath);
            if (!date) return `<url><loc>${url}</loc></url>`;
            dated += 1;
            if (!newest || date > newest) newest = date;
            return `<url><loc>${url}</loc><lastmod>${date}</lastmod></url>`;
          }
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
            `    <loc>${docsUrl("/sitemap.xml")}</loc>`,
            ...(newest ? [`    <lastmod>${newest}</lastmod>`] : []),
            "  </sitemap>",
            "</sitemapindex>",
            "",
          ].join("\n")
        );

        // Remove Starlight's intermediate sitemap files
        unlinkSync(sitemapSrc);

        const entryCount = (enhanced.match(/<url>/g) || []).length;
        console.log(
          `[sitemap-enhance] sitemap.xml and sitemap-index.xml written with ${entryCount} entries, ${dated} dated from git`
        );
      },
    },
  };
}
