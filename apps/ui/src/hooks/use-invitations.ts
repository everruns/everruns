"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { listInvites, createInvite, revokeInvite } from "@/lib/api/invitations";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { OrgRole } from "@/lib/api/types";

export function useInvitations() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.organizations.invitations(org ?? ""),
    queryFn: () => listInvites(org!),
    enabled: !!org,
    staleTime: 10000,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateInvite() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ email, role }: { email: string; role: OrgRole }) =>
      createInvite(org!, email, role),
    onSuccess: () => {
      if (org) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.invitations(org),
        });
      }
    },
  });
}

export function useRevokeInvite() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (inviteId: string) => revokeInvite(org!, inviteId),
    onSuccess: () => {
      if (org) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.invitations(org),
        });
      }
    },
  });
}
