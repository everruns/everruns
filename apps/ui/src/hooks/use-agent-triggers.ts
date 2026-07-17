"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgentTrigger,
  deleteAgentTrigger,
  listAgentTriggerRuns,
  listAgentTriggers,
  runAgentTrigger,
  updateAgentTrigger,
} from "@/lib/api/agent-triggers";
import type { CreateAgentTriggerRequest, UpdateAgentTriggerRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

export function useAgentTriggers(agentId: string) {
  return useQuery({
    queryKey: queryKeys.agentTriggers.list(agentId),
    queryFn: () => listAgentTriggers(agentId),
  });
}

export function useAgentTriggerRuns(agentId: string, triggerId: string) {
  return useQuery({
    queryKey: queryKeys.agentTriggers.runs(agentId, triggerId),
    queryFn: () => listAgentTriggerRuns(agentId, triggerId),
  });
}

function useInvalidateAgentTriggers(agentId: string) {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: queryKeys.agentTriggers.all(agentId) });
}

export function useCreateAgentTrigger(agentId: string) {
  const invalidate = useInvalidateAgentTriggers(agentId);
  return useMutation({
    mutationFn: (request: CreateAgentTriggerRequest) => createAgentTrigger(agentId, request),
    onSuccess: invalidate,
  });
}

export function useUpdateAgentTrigger(agentId: string) {
  const invalidate = useInvalidateAgentTriggers(agentId);
  return useMutation({
    mutationFn: ({
      triggerId,
      request,
    }: {
      triggerId: string;
      request: UpdateAgentTriggerRequest;
    }) => updateAgentTrigger(agentId, triggerId, request),
    onSuccess: invalidate,
  });
}

export function useDeleteAgentTrigger(agentId: string) {
  const invalidate = useInvalidateAgentTriggers(agentId);
  return useMutation({
    mutationFn: (triggerId: string) => deleteAgentTrigger(agentId, triggerId),
    onSuccess: invalidate,
  });
}

export function useRunAgentTrigger(agentId: string) {
  const invalidate = useInvalidateAgentTriggers(agentId);
  return useMutation({
    mutationFn: (triggerId: string) => runAgentTrigger(agentId, triggerId),
    onSuccess: invalidate,
  });
}
