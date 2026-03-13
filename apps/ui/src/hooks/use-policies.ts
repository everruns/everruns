"use client";

import { useQuery } from "@tanstack/react-query";
import { useCallback } from "react";
import { api } from "@/lib/api/client";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { ResourceConfigResponse } from "@/lib/api/types";

/**
 * Fetch and cache policy config for a resource.
 * Usage: const { can } = usePolicies("harnesses");
 *        if (can("harness.manage")) { ... }
 */
export function usePolicies(resource: string) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.policies.config(resource), org],
    queryFn: async () => {
      const response = await api.get<ResourceConfigResponse>(`/v1/${resource}/config`);
      return response.data;
    },
    enabled: !!org,
    staleTime: 5 * 60 * 1000, // 5 min
  });

  const can = useCallback(
    (policyId: string) => {
      return query.data?.policies?.[policyId] ?? false;
    },
    [query.data],
  );

  return { ...query, can };
}
