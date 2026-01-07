// Agent hooks (M2)

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
import type { CreateAgentRequest, UpdateAgentRequest } from "@/lib/api/types";

export function useAgents() {
  return useQuery({
    queryKey: ["agents"],
    queryFn: listAgents,
  });
}

export function useAgent(agentId: string | undefined) {
  return useQuery({
    queryKey: ["agent", agentId],
    queryFn: () => getAgent(agentId!),
    enabled: !!agentId,
  });
}

export function useCreateAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateAgentRequest) => createAgent(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
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
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      queryClient.invalidateQueries({ queryKey: ["agent", agentId] });
    },
  });
}

export function useDeleteAgent() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (agentId: string) => deleteAgent(agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
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
      queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
  });
}
