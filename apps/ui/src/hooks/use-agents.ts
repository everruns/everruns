// Agent hooks (M2)
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgent,
  deleteAgent,
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

export function useAgents() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.agents.list(), org],
    queryFn: () => listAgents(),
    enabled: !!org,
  });
}

export function useAgent(agentId: string | undefined) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.agents.detail(agentId!), org],
    queryFn: () => getAgent(agentId!),
    enabled: !!org && !!agentId,
  });
}

export function useCreateAgent() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

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
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request: UpdateAgentRequest;
    }) => updateAgent(agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) });
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
