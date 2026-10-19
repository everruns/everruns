import { legacyTabRedirect, type SearchParamsRecord } from "@/lib/tab-routes";

// Providers tab. Content lives in the shell (layout.tsx); this page redirects
// retired `/web-search?tab=…` links to the tab's own path.
export default async function WebSearchPage({
  searchParams,
}: {
  searchParams: Promise<SearchParamsRecord>;
}) {
  legacyTabRedirect("/web-search", await searchParams, ["providers", "defaults"], "providers");
  return null;
}
