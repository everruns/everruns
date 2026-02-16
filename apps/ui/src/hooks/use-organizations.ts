"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { createOrganization, getOrganization, updateOrganization } from "@/lib/api/organizations";
import { queryKeys } from "@/lib/query-keys";
import type { CreateOrganizationRequest, UpdateOrganizationRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export function useOrganization() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.organizations.detail(org ?? ""), org],
    queryFn: () => getOrganization(org!),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateOrganization() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateOrganizationRequest) => createOrganization(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
      // Refresh user's org list so the new org appears in the dropdown
      queryClient.invalidateQueries({ queryKey: queryKeys.users.me() });
    },
  });
}

export function useUpdateOrganization() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: UpdateOrganizationRequest) => updateOrganization(org!, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
      if (org) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.detail(org),
        });
      }
      // Also invalidate auth session to refresh user's org list
      queryClient.invalidateQueries({ queryKey: queryKeys.users.me() });
    },
  });
}
