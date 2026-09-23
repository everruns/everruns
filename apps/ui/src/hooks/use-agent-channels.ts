"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgentChannel,
  deleteAgentChannel,
  listAgentChannels,
  publishAgentChannel,
  triggerAgentChannel,
  unpublishAgentChannel,
  updateAgentChannel,
  type CreateAgentChannelRequest,
  type UpdateAgentChannelRequest,
} from "@/lib/api/agent-channels";
import type { AppChannel } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

export interface AgentChannel {
  channel: AppChannel;
}

/// Which channel types are exposures (a door traffic arrives through) versus
/// triggers (something that fires on its own). The Integrations tab shows both,
/// in separate sections, because they answer different questions: "how do I
/// reach this agent" and "when does it wake up on its own".
const TRIGGER_CHANNEL_TYPES = new Set(["schedule"]);

export function isTriggerChannel(channel: AppChannel): boolean {
  return TRIGGER_CHANNEL_TYPES.has(channel.channel_type);
}

export function useAgentChannels(agentId: string | undefined) {
  const query = useQuery({
    queryKey: queryKeys.agentChannels.list(agentId ?? ""),
    queryFn: () => listAgentChannels(agentId as string),
    enabled: !!agentId,
  });
  return {
    ...query,
    channels: (query.data ?? []).map((channel) => ({ channel })),
  };
}

function useChannelMutation<TVariables, TResult>(
  agentId: string,
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agentChannels.all(agentId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) });
    },
  });
}

export function useCreateAgentChannel(agentId: string) {
  return useChannelMutation(agentId, (request: CreateAgentChannelRequest) =>
    createAgentChannel(agentId, request),
  );
}

export function useUpdateAgentChannel(agentId: string, channelId: string) {
  return useChannelMutation(agentId, (request: UpdateAgentChannelRequest) =>
    updateAgentChannel(agentId, channelId, request),
  );
}

export function useDeleteAgentChannel(agentId: string, channelId: string) {
  return useChannelMutation(agentId, () => deleteAgentChannel(agentId, channelId));
}

export function usePublishAgentChannel(agentId: string) {
  return useChannelMutation(
    agentId,
    ({ channelId, publish }: { channelId: string; publish: boolean }) =>
      publish ? publishAgentChannel(agentId, channelId) : unpublishAgentChannel(agentId, channelId),
  );
}

export function useTriggerAgentChannel(agentId: string) {
  return useChannelMutation(agentId, (channelId: string) =>
    triggerAgentChannel(agentId, channelId),
  );
}
