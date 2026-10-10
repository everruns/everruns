"use client";

import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createAgentChannel,
  createAgentKey,
  deleteAgentChannel,
  getSlackInstallCapability,
  listAgentChannels,
  listAgentKeys,
  listSlackWorkspaces,
  publishAgentChannel,
  revokeAgentKey,
  rotateAgentKey,
  triggerAgentChannel,
  unpublishAgentChannel,
  updateAgentChannel,
  type CreateAgentChannelRequest,
  type UpdateAgentChannelRequest,
} from "@/lib/api/agent-channels";
import type { AgentChannel } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

export interface AgentChannelRow {
  channel: AgentChannel;
}

/// Which channel types are exposures (a door traffic arrives through) versus
/// triggers (something that fires on its own). The Integrations tab shows both,
/// in separate sections, because they answer different questions: "how do I
/// reach this agent" and "when does it wake up on its own".
const TRIGGER_CHANNEL_TYPES = new Set(["schedule"]);

export function isTriggerChannel(channel: AgentChannel): boolean {
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
      queryClient.invalidateQueries({
        queryKey: queryKeys.agentChannels.all(agentId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.agents.detail(agentId),
      });
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}

export function useCreateAgentChannel(agentId: string) {
  return useChannelMutation(agentId, (request: CreateAgentChannelRequest) =>
    createAgentChannel(agentId, request),
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

export function useAgentKeys(agentId: string, channelId: string) {
  return useQuery({
    queryKey: queryKeys.agentChannels.keys(agentId, channelId),
    queryFn: () => listAgentKeys(agentId, channelId),
  });
}

function useAgentKeyMutation<TVariables, TResult>(
  agentId: string,
  channelId: string,
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: queryKeys.agentChannels.keys(agentId, channelId),
      }),
  });
}

export function useCreateAgentKey(agentId: string, channelId: string) {
  return useAgentKeyMutation(agentId, channelId, (name: string) =>
    createAgentKey(agentId, channelId, name),
  );
}

export function useRotateAgentKey(agentId: string, channelId: string) {
  return useAgentKeyMutation(
    agentId,
    channelId,
    ({ keyId, overlapHours }: { keyId: string; overlapHours: number }) =>
      rotateAgentKey(agentId, channelId, keyId, overlapHours),
  );
}

export function useRevokeAgentKey(agentId: string, channelId: string) {
  return useAgentKeyMutation(agentId, channelId, (keyId: string) =>
    revokeAgentKey(agentId, channelId, keyId),
  );
}
