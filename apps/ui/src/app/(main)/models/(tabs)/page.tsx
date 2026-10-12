import { legacyTabRedirect, type SearchParamsRecord } from "@/lib/tab-routes";

// Models tab. Content lives in the shell (layout.tsx); this page redirects
// retired `/models?tab=…` links to the tab's own path.
export default async function ModelsPage({
  searchParams,
}: {
  searchParams: Promise<SearchParamsRecord>;
}) {
  legacyTabRedirect("/models", await searchParams, ["models", "providers", "defaults"], "models");
  return null;
}
