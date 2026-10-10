"use client";

import { useCallback, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useCapabilities, useCreateAgent } from "@/hooks";
import { createAgentChannel, type CreateAgentChannelRequest } from "@/lib/api/agent-channels";
import { createAgentTrigger } from "@/lib/api/agent-triggers";
import type { AgentDraft } from "@/lib/api/agent-draft";
import type { AgentCapabilityConfig, Capability, CreateAgentTriggerRequest } from "@/lib/api/types";
import { generateChannelToken } from "@/lib/channel-tokens";
import { DEFAULT_WEBHOOK_MESSAGE } from "@/lib/new-agent";
import { queryKeys } from "@/lib/query-keys";
import {
  buildChannelConfig,
  getDefaultChannelFormState,
} from "@/components/agents/channels/channel-form";

/** Channel create requests for a draft. Slack is set up afterwards on its own form. */
export function draftChannelRequests(draft: AgentDraft): CreateAgentChannelRequest[] {
  const requests: CreateAgentChannelRequest[] = [];
  for (const kind of draft.channels) {
    if (kind === "slack") continue;
    const state = getDefaultChannelFormState(kind);
    if (kind === "webhook") {
      // The server requires a token; the channel page shows it once published.
      state.webhookToken = generateChannelToken();
      state.channelMessage = DEFAULT_WEBHOOK_MESSAGE;
    }
    requests.push({
      channel_type: kind,
      channel_config: buildChannelConfig(state),
      enabled: true,
    });
  }
  return requests;
}

/** Chosen capabilities plus their dependencies, dependencies first, as the selector adds them. */
export function draftCapabilityConfigs(
  ids: string[],
  capabilities: Capability[] | undefined,
): AgentCapabilityConfig[] {
  const ordered: string[] = [];
  const visit = (id: string, seen: Set<string>) => {
    if (ordered.includes(id) || seen.has(id)) return;
    seen.add(id);
    for (const dep of capabilities?.find((c) => c.id === id)?.dependencies ?? []) visit(dep, seen);
    ordered.push(id);
  };
  for (const id of ids) visit(id, new Set());
  return ordered.map((ref) => ({ ref, config: {} }));
}

/** The draft's schedule as a schedule trigger (schedules are triggers, not channels). */
export function draftScheduleTrigger(draft: AgentDraft): CreateAgentTriggerRequest | null {
  if (!draft.schedule) return null;
  return {
    trigger_type: "schedule",
    cron_expression: draft.schedule.cron,
    timezone: draft.schedule.timezone || "UTC",
    session_mode: "shared_session",
    message: draft.schedule.message,
  };
}

export interface CreatedFromDraft {
  agentId: string;
  /** Ways in (or the schedule) that could not be created; the agent exists regardless. */
  failedChannels: string[];
}

/** Create the agent, then its draft channels and schedule. */
export function useCreateAgentFromDraft() {
  const createAgent = useCreateAgent();
  const { data: capabilities } = useCapabilities();
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(false);

  const create = useCallback(
    async (draft: AgentDraft): Promise<CreatedFromDraft> => {
      setPending(true);
      try {
        const agent = await createAgent.mutateAsync({
          name: draft.name,
          display_name: draft.display_name.trim(),
          description: draft.description.trim() || undefined,
          system_prompt: draft.system_prompt,
          capabilities:
            draft.capabilities.length > 0
              ? draftCapabilityConfigs(draft.capabilities, capabilities)
              : undefined,
        });
        const failedChannels: string[] = [];
        for (const request of draftChannelRequests(draft)) {
          try {
            await createAgentChannel(agent.id, request);
          } catch (error) {
            console.error("Failed to create draft channel:", error);
            failedChannels.push(request.channel_type);
          }
        }
        const schedule = draftScheduleTrigger(draft);
        if (schedule) {
          try {
            await createAgentTrigger(agent.id, schedule);
          } catch (error) {
            console.error("Failed to create the schedule:", error);
            failedChannels.push("schedule");
          }
        }
        await queryClient.invalidateQueries({
          queryKey: queryKeys.agentChannels.all(agent.id),
        });
        await queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
        return { agentId: agent.id, failedChannels };
      } finally {
        setPending(false);
      }
    },
    [capabilities, createAgent, queryClient],
  );

  return { create, pending, error: createAgent.error };
}
