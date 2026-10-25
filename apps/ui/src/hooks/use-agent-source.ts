"use client";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { queryKeys } from "@/lib/query-keys";
import { getAgentSource } from "@/lib/api/agent-source";

export const agentSourceKey = (id: string) => ["agent-source", id] as const;
export function useAgentSource(id: string) {
  const client = useQueryClient();
  const lastCommit = useRef<string | null | undefined>(undefined);
  const query = useOrgScopedQuery({
    queryKey: agentSourceKey(id),
    queryFn: () => getAgentSource(id),
    refetchInterval: (query) =>
      query.state.data?.status === "pending" || query.state.data?.status === "syncing"
        ? 2000
        : 10000,
  });
  useEffect(() => {
    const commit = query.data?.applied_commit;
    if (lastCommit.current !== undefined && lastCommit.current !== commit) {
      client.invalidateQueries({ queryKey: queryKeys.agents.detail(id) });
      client.invalidateQueries({ queryKey: queryKeys.agentChannels.all(id) });
    }
    lastCommit.current = commit;
  }, [query.data?.applied_commit, client, id]);
  return query;
}
