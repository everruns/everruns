import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const docsAppRoot = path.resolve(scriptDir, "..");
const distRoot = path.join(docsAppRoot, "dist");
const manifest = JSON.parse(
  readFileSync(path.join(docsAppRoot, "src/generated/notebooks/manifest.json"), "utf8")
);
const sitemap = readFileSync(path.join(distRoot, "sitemap.xml"), "utf8");
const sitemapIndexPath = path.join(distRoot, "sitemap-index.xml");

if (!existsSync(sitemapIndexPath)) {
  throw new Error(`sitemap-index.xml missing from build output: ${sitemapIndexPath}`);
}

const sitemapIndex = readFileSync(sitemapIndexPath, "utf8");
if (!sitemapIndex.includes("<loc>https://docs.everruns.com/sitemap.xml</loc>")) {
  throw new Error("sitemap-index.xml does not reference sitemap.xml");
}

for (const entry of Object.values(manifest)) {
  const routePath = entry.route.replace(/^\/+/, "");
  const assetPath = entry.publicHref.replace(/^\/+/, "");
  const pagePath = path.join(distRoot, routePath, "index.html");
  const notebookAssetPath = path.join(distRoot, assetPath);

  if (!existsSync(pagePath)) {
    throw new Error(`Notebook-backed docs page missing from build output: ${pagePath}`);
  }

  if (!existsSync(notebookAssetPath)) {
    throw new Error(`Notebook download asset missing from build output: ${notebookAssetPath}`);
  }

  if (!sitemap.includes(`<loc>https://docs.everruns.com${entry.route}</loc>`)) {
    throw new Error(`Notebook-backed docs page missing from sitemap.xml: ${entry.route}`);
  }
}

console.log(
  `Verified ${Object.keys(manifest).length} notebook-backed docs page(s) in build output, sitemap.xml, and sitemap-index.xml.`
);
