"use client";

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import type { AgentFailureSessions } from "@/lib/api/schema-types";
import type { Session } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Use the same UI Session view as listSessions/getSession; retain generated
// types for the endpoint's failure metadata and pagination.
type FailureSessions = Omit<AgentFailureSessions, "data"> & {
  data: Array<Omit<AgentFailureSessions["data"][number], "session"> & { session: Session }>;
};

export function useAgentFailures(agentId: string, after: string, before: string, offset: number) {
  const { currentOrg, isLoading } = useOrg();
  const query = useQuery({
    queryKey: ["agent-failures", currentOrg?.public_id, agentId, after, before, offset],
    queryFn: async () => {
      const params = new URLSearchParams({ after, before, offset: String(offset), limit: "20" });
      return (await api.get<FailureSessions>(`/v1/agents/${agentId}/failures?${params}`)).data;
    },
    enabled: !!currentOrg,
  });
  return { ...query, isLoading: isLoading || query.isLoading };
}
