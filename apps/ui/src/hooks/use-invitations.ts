"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  acceptPendingInvitation,
  createInvite,
  listInvites,
  listPendingInvitations,
  revokeInvite,
} from "@/lib/api/invitations";
import { switchOrg } from "@/lib/api/users";
import { queryKeys } from "@/lib/query-keys";
import { authKeys } from "@/hooks/use-auth";
import { useOrg } from "@/providers/org-provider";
import type { OrgRole } from "@/lib/api/types";

export function useInvitations() {
  const { currentOrg, hasRole, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  // GET /v1/orgs/:org/invites is OrgAdmin-only, so members would fetch a
  // guaranteed 403 on every render of the members page.
  const canManage = hasRole("admin");

  const query = useQuery({
    queryKey: queryKeys.organizations.invitations(org ?? ""),
    queryFn: () => listInvites(org!),
    enabled: !!org && canManage,
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

export function usePendingInvitations(enabled = true) {
  return useQuery({
    queryKey: queryKeys.invitations.pending(),
    queryFn: listPendingInvitations,
    enabled,
    retry: false,
  });
}

export function useAcceptPendingInvitation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (inviteId: string) => {
      const result = await acceptPendingInvitation(inviteId);
      try {
        await switchOrg(result.org_id);
      } catch {
        // Match the invite-link flow: membership is active even when the
        // preference cookie cannot be updated, so refresh the org list.
      }
      await queryClient.refetchQueries({ queryKey: authKeys.user() });
      return result;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.invitations.pending() });
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
