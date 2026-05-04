// Agent hooks (M2)
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  agentsCrudApi,
  copyAgent,
  exportAgent,
  getAgentStats,
  importAgent,
  previewAgent,
} from "@/lib/api/agents";
import type { CreateAgentRequest, PreviewAgentRequest, UpdateAgentRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";
import { useOrg } from "@/providers/org-provider";

const agentCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof agentsCrudApi.get>>,
  CreateAgentRequest,
  UpdateAgentRequest
>({
  api: agentsCrudApi,
  queryKeys: queryKeys.agents,
});

export const useAgents = agentCrudHooks.useList;
export const useAgent = agentCrudHooks.useDetail;
export const useCreateAgent = agentCrudHooks.useCreate;
export const useDeleteAgent = agentCrudHooks.useDelete;
export const useDestroyAgent = agentCrudHooks.useDestroy;

export function useAgentStats(agentId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.agents.stats(org, agentId),
    queryFn: () => getAgentStats(agentId!),
    enabled: !!org && !!agentId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useUpdateAgent() {
  const mutation = agentCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { agentId: string; request: UpdateAgentRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.agentId, request: variables.request }, options),
    mutateAsync: (
      variables: { agentId: string; request: UpdateAgentRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.agentId, request: variables.request }, options),
  };
}

export function useCopyAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: copyAgent,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useExportAgent() {
  return useMutation({
    mutationFn: exportAgent,
  });
}

export function useImportAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: importAgent,
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
