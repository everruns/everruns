// Agent hooks (M2)
"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { agentsCrudApi, copyAgent, exportAgent, importAgent, previewAgent } from "@/lib/api/agents";
import type { CreateAgentRequest, PreviewAgentRequest, UpdateAgentRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";

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
