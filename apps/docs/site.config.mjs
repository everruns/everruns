// Where the docs are served: everruns.com/docs/. One place for the build,
// its integrations, and the pre/post-build scripts.
//
// Decision: the docs share the everruns.com domain with the landing site so
// search ranking and the brand stay on one domain. docs.everruns.com 301s
// every path to the same path under /docs (knowledge/ui/documentation.md).
export const SITE = "https://everruns.com";
export const BASE = "/docs";

/** `/changelog/` → `/docs/changelog/`; external and already-prefixed URLs pass through. */
export function withBase(url, base = BASE) {
  const prefix = base.replace(/\/+$/, "");
  if (!prefix || !url.startsWith("/") || url.startsWith("//")) return url;
  if (url === prefix || url.startsWith(`${prefix}/`)) return url;
  return `${prefix}${url}`;
}

/** The absolute URL of a docs path: `/changelog/` → `https://everruns.com/docs/changelog/`. */
export const docsUrl = (path) => `${SITE}${withBase(path)}`;
