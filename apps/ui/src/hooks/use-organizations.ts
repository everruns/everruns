"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { createOrganization, getOrganization, updateOrganization } from "@/lib/api/organizations";
import { queryKeys } from "@/lib/query-keys";
import { authKeys } from "@/hooks/use-auth";
import type { UpdateOrganizationRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export function useOrganization() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.organizations.detail(org ?? ""),
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
    mutationFn: createOrganization,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all }),
        // Refresh user's org list so the new org appears in the sidebar dropdown.
        // The org list is sourced from the auth user query (["auth", "user"]),
        // not queryKeys.users.me, so we must invalidate authKeys.user().
        // Awaiting ensures the org list is up-to-date before mutateAsync resolves,
        // preventing race conditions where setCurrentOrg runs before the new org
        // appears in the organizations array (which would reset to the old org).
        queryClient.invalidateQueries({ queryKey: authKeys.user() }),
      ]);
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
      // Refresh auth user query so the sidebar org dropdown reflects updates
      queryClient.invalidateQueries({ queryKey: authKeys.user() });
    },
  });
}
