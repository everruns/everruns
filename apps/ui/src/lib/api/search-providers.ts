// Web search providers of the active organization.
// Routes: GET /v1/search-providers, POST /v1/search-providers/preview.
// Org is sent via the everruns_org cookie, like the other org-scoped reads.

import { api } from "./client";
import type { SearchPlanPreview, SearchPlanPreviewRequest, SearchProviderCatalog } from "./types";

export async function listSearchProviders(): Promise<SearchProviderCatalog> {
  return (await api.get<SearchProviderCatalog>("/v1/search-providers")).data;
}

export async function previewSearchPlan(
  request: SearchPlanPreviewRequest,
): Promise<SearchPlanPreview> {
  return (await api.post<SearchPlanPreview>("/v1/search-providers/preview", request)).data;
}
