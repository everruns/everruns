import { BASE } from "./site.config.mjs";

// Keep the shared announcement URL durable without indexing it as a second page.
export function changelogRedirect(url) {
  const from = `${BASE}/changelog/2026-10-28-agent-cost-estimation`;
  if (![from, `${from}/`, `${from}.md`, `${from}/index.md`].includes(url.pathname)) return null;
  const suffix = url.pathname.endsWith(".md") ? "/index.md" : "/";
  return new URL(`${BASE}/changelog/2026-10-28-smart-agent-cost-projections${suffix}${url.search}`, url);
}
