"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgentIdentity,
  deleteAgentIdentity,
  destroyAgentIdentity,
  getAgentIdentity,
  listAgentIdentities,
  updateAgentIdentity,
} from "@/lib/api/agent-identities";
import { useOrg } from "@/providers/org-provider";
import type {
  CreateAgentIdentityRequest,
  UpdateAgentIdentityRequest,
} from "@/lib/api/types";

const agentIdentityKeys = {
  all: ["agent-identities"] as const,
  list: (includeArchived: boolean) => ["agent-identities", "list", includeArchived] as const,
  detail: (id: string) => ["agent-identities", "detail", id] as const,
};

export function useAgentIdentities(options: { includeArchived?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const includeArchived = options.includeArchived ?? false;
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...agentIdentityKeys.list(includeArchived), org],
    queryFn: () => listAgentIdentities(includeArchived),
    enabled: !!org,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}

export function useAgentIdentity(identityId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...agentIdentityKeys.detail(identityId ?? ""), org],
    queryFn: () => getAgentIdentity(identityId!),
    enabled: !!org && !!identityId,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}

export function useCreateAgentIdentity() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateAgentIdentityRequest) => createAgentIdentity(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: agentIdentityKeys.all });
    },
  });
}

export function useUpdateAgentIdentity() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ identityId, request }: { identityId: string; request: UpdateAgentIdentityRequest }) =>
      updateAgentIdentity(identityId, request),
    onSuccess: (identity) => {
      queryClient.invalidateQueries({ queryKey: agentIdentityKeys.all });
      queryClient.setQueryData(agentIdentityKeys.detail(identity.id), identity);
    },
  });
}

export function useDeleteAgentIdentity() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (identityId: string) => deleteAgentIdentity(identityId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: agentIdentityKeys.all });
    },
  });
}

export function useDestroyAgentIdentity() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (identityId: string) => destroyAgentIdentity(identityId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: agentIdentityKeys.all });
    },
  });
}
