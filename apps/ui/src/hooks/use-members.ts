"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { listMembers, updateMemberRole, removeMember } from "@/lib/api/members";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { OrgRole } from "@/lib/api/types";

export function useMembers() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.organizations.members(org ?? ""),
    queryFn: () => listMembers(org!),
    enabled: !!org,
    staleTime: 10000,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useUpdateMemberRole() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: OrgRole }) =>
      updateMemberRole(org!, userId, role),
    onSuccess: () => {
      if (org) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.members(org),
        });
      }
    },
  });
}

export function useRemoveMember() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (userId: string) => removeMember(org!, userId),
    onSuccess: () => {
      if (org) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.members(org),
        });
      }
    },
  });
}
