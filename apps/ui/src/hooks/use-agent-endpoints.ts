"use client";

import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgentEndpoint,
  deleteAgentEndpoint,
  getSlackInstallCapability,
  listAgentEndpoints,
  listSlackWorkspaces,
  publishAgentEndpoint,
  triggerAgentEndpoint,
  unpublishAgentEndpoint,
  updateAgentEndpoint,
  type CreateAgentEndpointRequest,
  type UpdateAgentEndpointRequest,
} from "@/lib/api/agent-endpoints";
import type { AgentEndpoint } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

export interface AgentEndpointRow {
  channel: AgentEndpoint;
}

/// Which channel types are exposures (a door traffic arrives through) versus
/// triggers (something that fires on its own). The Integrations tab shows both,
/// in separate sections, because they answer different questions: "how do I
/// reach this agent" and "when does it wake up on its own".
const TRIGGER_CHANNEL_TYPES = new Set(["schedule"]);

export function isTriggerChannel(channel: AgentEndpoint): boolean {
  return TRIGGER_CHANNEL_TYPES.has(channel.channel_type);
}

export function useAgentEndpoints(agentId: string | undefined) {
  const query = useQuery({
    queryKey: queryKeys.agentEndpoints.list(agentId ?? ""),
    queryFn: () => listAgentEndpoints(agentId as string),
    enabled: !!agentId,
  });
  return {
    ...query,
    endpoints: (query.data ?? []).map((channel) => ({ channel })),
  };
}

function useEndpointMutation<TVariables, TResult>(
  agentId: string,
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.agentEndpoints.all(agentId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.agents.detail(agentId),
      });
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useCreateAgentEndpoint(agentId: string) {
  return useEndpointMutation(agentId, (request: CreateAgentEndpointRequest) =>
    createAgentEndpoint(agentId, request),
  );
}
export function useSlackInstallCapability(enabled = true) {
  return useQuery({
    queryKey: ["slack-install-capability"],
    queryFn: getSlackInstallCapability,
    enabled,
  });
}

export const SLACK_WORKSPACES_QUERY_KEY = ["slack-workspaces"] as const;

export function useSlackWorkspaces(enabled = true) {
  return useQuery({
    queryKey: SLACK_WORKSPACES_QUERY_KEY,
    queryFn: listSlackWorkspaces,
    enabled,
  });
}

/** Refresh everything that depends on which workspaces are connected. */
export function useInvalidateSlackWorkspaces() {
  const queryClient = useQueryClient();
  return useCallback(
    () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: SLACK_WORKSPACES_QUERY_KEY }),
        queryClient.invalidateQueries({
          queryKey: ["slack-install-capability"],
        }),
      ]),
    [queryClient],
  );
}

export function useUpdateAgentEndpoint(agentId: string, endpointId: string) {
  return useEndpointMutation(agentId, (request: UpdateAgentEndpointRequest) =>
    updateAgentEndpoint(agentId, endpointId, request),
  );
}

export function useDeleteAgentEndpoint(agentId: string, endpointId: string) {
  return useEndpointMutation(agentId, () => deleteAgentEndpoint(agentId, endpointId));
}

export function usePublishAgentEndpoint(agentId: string) {
  return useEndpointMutation(
    agentId,
    ({ endpointId, publish }: { endpointId: string; publish: boolean }) =>
      publish
        ? publishAgentEndpoint(agentId, endpointId)
        : unpublishAgentEndpoint(agentId, endpointId),
  );
}

export function useTriggerAgentEndpoint(agentId: string) {
  return useEndpointMutation(agentId, (endpointId: string) =>
    triggerAgentEndpoint(agentId, endpointId),
  );
}
