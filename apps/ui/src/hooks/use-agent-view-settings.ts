"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getAgentViewSettings,
  getSessionAgentView,
  replaceAgentView,
  saveSessionAgentView,
  type AgentViewSettings,
} from "@/lib/api/agent-views";
import type { AgentViewLayout } from "@everruns/generative-ui";
import { queryKeys } from "@/lib/query-keys";

export function useAgentViewSettings(agentId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.agents.viewSettings(agentId),
    queryFn: () => getAgentViewSettings(agentId),
    enabled,
  });
}

export function useReplaceAgentView(agentId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: AgentViewSettings) => replaceAgentView(agentId, settings),
    onSuccess: (settings) => {
      queryClient.setQueryData(queryKeys.agents.viewSettings(agentId), settings);
    },
  });
}

export function useSessionAgentView(sessionId: string | undefined, agentId?: string) {
  const queryClient = useQueryClient();
  const key = ["session", sessionId, "agent-view", agentId] as const;
  const query = useQuery({
    queryKey: key,
    queryFn: () => getSessionAgentView(sessionId!, agentId),
    enabled: !!sessionId,
    retry: false,
    refetchInterval: 10_000,
  });
  return {
    view: sessionId && !query.isError ? query.data : null,
    async save(revision: number, layout: AgentViewLayout | null) {
      if (!sessionId) return;
      const saved = await saveSessionAgentView(sessionId, revision, layout, agentId);
      queryClient.setQueryData(key, saved);
    },
  };
}
