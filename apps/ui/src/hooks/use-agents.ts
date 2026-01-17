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
  updateAgent,
} from "@/lib/api/agents";
import { queryKeys } from "@/lib/query-keys";
import type { CreateAgentRequest, UpdateAgentRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export function useAgents() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.agents.list(), org],
    queryFn: () => listAgents(org!),
    enabled: !!org,
  });
}

export function useAgent(agentId: string | undefined) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.agents.detail(agentId!), org],
    queryFn: () => getAgent(org!, agentId!),
    enabled: !!org && !!agentId,
  });
}

export function useCreateAgent() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (request: CreateAgentRequest) => createAgent(org!, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useUpdateAgent() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request: UpdateAgentRequest;
    }) => updateAgent(org!, agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) });
    },
  });
}

export function useDeleteAgent() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (agentId: string) => deleteAgent(org!, agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useExportAgent() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (agentId: string) => exportAgent(org!, agentId),
  });
}

export function useImportAgent() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (markdown: string) => importAgent(org!, markdown),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}
