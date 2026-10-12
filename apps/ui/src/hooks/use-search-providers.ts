"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { listSearchProviders, previewSearchPlan } from "@/lib/api/search-providers";
import { updateOrganization } from "@/lib/api/organizations";
import type { SearchPlanPreviewRequest, WebSearchDefaults } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

const catalogKey = (org?: string) => ["search-providers", org] as const;

/** Built-ins, native-capable model providers, connectors, accounts and defaults. */
export function useSearchProviders() {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: catalogKey(currentOrg?.public_id),
    enabled: !!currentOrg,
    queryFn: listSearchProviders,
  });
}

/** The plan an agent's web search setting resolves to for a model. */
export function useSearchPlanPreview(request: SearchPlanPreviewRequest | null) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["search-providers", currentOrg?.public_id, "preview", request],
    enabled: !!currentOrg && request !== null,
    queryFn: () => previewSearchPlan(request!),
    staleTime: 15_000,
  });
}

/** Save the organization's web search defaults (organization admins). */
export function useSaveWebSearchDefaults() {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (defaults: WebSearchDefaults) =>
      updateOrganization(currentOrg!.public_id, { web_search_defaults: defaults }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["search-providers"] });
      if (currentOrg) {
        await client.invalidateQueries({
          queryKey: queryKeys.organizations.detail(currentOrg.public_id),
        });
      }
    },
  });
}

/** Refresh the catalog after an account is connected, rotated or removed. */
export function useInvalidateSearchProviders() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: ["search-providers"] });
}
