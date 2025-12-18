// Capability hooks

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getAgentCapabilities,
  getCapability,
  listCapabilities,
  setAgentCapabilities,
} from "@/lib/api/capabilities";
import type { CapabilityId, UpdateAgentCapabilitiesRequest } from "@/lib/api/types";

export function useCapabilities() {
  return useQuery({
    queryKey: ["capabilities"],
    queryFn: listCapabilities,
  });
}

export function useCapability(capabilityId: CapabilityId | undefined) {
  return useQuery({
    queryKey: ["capability", capabilityId],
    queryFn: () => getCapability(capabilityId!),
    enabled: !!capabilityId,
  });
}

export function useAgentCapabilities(agentId: string | undefined) {
  return useQuery({
    queryKey: ["agent-capabilities", agentId],
    queryFn: () => getAgentCapabilities(agentId!),
    enabled: !!agentId,
  });
}

export function useSetAgentCapabilities() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request: UpdateAgentCapabilitiesRequest;
    }) => setAgentCapabilities(agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: ["agent-capabilities", agentId] });
    },
  });
}
