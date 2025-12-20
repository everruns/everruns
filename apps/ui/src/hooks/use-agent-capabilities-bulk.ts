// Hook to fetch capabilities for multiple agents in parallel

import { useQueries } from "@tanstack/react-query";
import { getAgentCapabilities } from "@/lib/api/capabilities";
import type { AgentCapability } from "@/lib/api/types";

export function useAgentCapabilitiesBulk(agentIds: string[]) {
  const queries = useQueries({
    queries: agentIds.map((agentId) => ({
      queryKey: ["agent-capabilities", agentId],
      queryFn: () => getAgentCapabilities(agentId),
      enabled: !!agentId,
    })),
  });

  // Build a map of agentId -> capabilities
  const data: Record<string, AgentCapability[]> = {};
  const isLoading = queries.some((q) => q.isLoading);
  const isError = queries.some((q) => q.isError);

  agentIds.forEach((agentId, index) => {
    const result = queries[index];
    if (result.data) {
      data[agentId] = result.data;
    }
  });

  return {
    data: Object.keys(data).length > 0 ? data : undefined,
    isLoading,
    isError,
  };
}
