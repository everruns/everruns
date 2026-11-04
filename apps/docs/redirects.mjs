import { BASE } from "./site.config.mjs";
import { GALLERY_ROOT, LEGACY_GALLERY_ROOT } from "./generative-ui-catalog.mjs";

// Keep the shared announcement URL durable without indexing it as a second page.
export function changelogRedirect(url) {
  const from = `${BASE}/changelog/2026-10-28-agent-cost-estimation`;
  if (![from, `${from}/`, `${from}.md`, `${from}/index.md`].includes(url.pathname)) return null;
  const suffix = url.pathname.endsWith(".md") ? "/index.md" : "/";
  return new URL(`${BASE}/changelog/2026-10-28-smart-agent-cost-projections${suffix}${url.search}`, url);
}

// Preserve both browser bookmarks and agent-readable Markdown links.
export function galleryRedirect(url) {
  const from = `${BASE}${LEGACY_GALLERY_ROOT}`;
  if (![from, `${from}/`, `${from}.md`, `${from}/index.md`].includes(url.pathname)) return null;
  const suffix = url.pathname.endsWith(".md") ? "/index.md" : "/";
  return new URL(`${BASE}${GALLERY_ROOT}${suffix}${url.search}`, url);
}
