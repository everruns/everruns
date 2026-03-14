// Agent hooks (M2)
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  copyAgent,
  createAgent,
  deleteAgent,
  destroyAgent,
  exportAgent,
  getAgent,
  importAgent,
  listAgents,
  previewAgent,
  updateAgent,
} from "@/lib/api/agents";
import { queryKeys } from "@/lib/query-keys";
import type { CreateAgentRequest, PreviewAgentRequest, UpdateAgentRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

interface UseAgentsOptions {
  includeArchived?: boolean;
}

export function useAgents(options: UseAgentsOptions = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const includeArchived = options.includeArchived ?? false;

  const query = useQuery({
    queryKey: [...queryKeys.agents.list(includeArchived), org],
    queryFn: () => listAgents(includeArchived),
    enabled: !!org,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useAgent(agentId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.agents.detail(agentId!), org],
    queryFn: () => getAgent(agentId!),
    enabled: !!org && !!agentId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateAgentRequest) => createAgent(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useUpdateAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ agentId, request }: { agentId: string; request: UpdateAgentRequest }) =>
      updateAgent(agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.agents.detail(agentId),
      });
    },
  });
}

export function useDeleteAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (agentId: string) => deleteAgent(agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useDestroyAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (agentId: string) => destroyAgent(agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useCopyAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (agentId: string) => copyAgent(agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useExportAgent() {
  return useMutation({
    mutationFn: (agentId: string) => exportAgent(agentId),
  });
}

export function useImportAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (markdown: string) => importAgent(markdown),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function usePreviewAgent() {
  return useMutation({
    mutationFn: (request: PreviewAgentRequest) => previewAgent(request),
  });
}
