"use client";

import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { useAgents } from "./use-agents";
import { isTriggerChannel } from "./use-agent-channels";
import { listAgentChannels } from "@/lib/api/agent-channels";
import { queryKeys } from "@/lib/query-keys";
import type {
  Agent,
  AgUiChannelConfig,
  AgentChannel,
  FcpChannelConfig,
  PublicChatChannelConfig,
} from "@/lib/api/types";

/// Why an exposure is not accepting traffic, or that it is.
///
/// This mirrors `channel_liveness` in `crates/server/src/api/channel_ingress.rs`:
///
/// ```text
/// live = channel.status == live && agent.status == active && !agent.exposures_suspended
/// ```
///
/// The agent-level terms are folded in here rather than read off the channel,
/// exactly as the server folds them in at resolution time. A view that showed
/// `channel.status` alone would call a channel live while its agent is
/// suspended or archived — which is the one thing this page must never do,
/// because it is the page someone checks during an incident.
export type ExposureState = "live" | "draft" | "disabled" | "suspended" | "agent-inactive";

export function exposureStateLabel(state: ExposureState): string {
  switch (state) {
    case "live":
      return "Live";
    case "draft":
      return "Draft";
    case "disabled":
      return "Disabled";
    case "suspended":
      return "Suspended";
    case "agent-inactive":
      return "Agent inactive";
  }
}

export function resolveExposureState(
  channel: AgentChannel,
  agent: Agent | undefined,
): ExposureState {
  // No agent means nothing can serve the traffic, which the server reports as
  // `NoAgent`. Treat it the same way an archived agent is treated.
  if (!agent || agent.status !== "active") return "agent-inactive";
  if (agent.exposures_suspended) return "suspended";
  if (!channel.enabled || channel.status === "disabled") return "disabled";
  if (channel.status === "live") return "live";
  return "draft";
}

/// Whether anyone on the internet can reach this exposure without a credential.
///
/// This is the row an operator scans for. It is deliberately conservative: a
/// transport whose config we cannot read counts as *not* known-anonymous rather
/// than silently clean, so a parse failure never hides a public surface.
export function isAnonymousExposure(channel: AgentChannel): boolean {
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    const auth = channel.auth ?? config.auth;
    const hasSignIn = !!auth && auth.mode !== "anonymous";
    const tokenProtected = !!config.token_configured || !!config.token;
    return !hasSignIn && !tokenProtected && config.anonymous !== false;
  }
  if (channel.channel_type === "ag_ui") {
    const config = channel.channel_config as AgUiChannelConfig;
    const auth = channel.auth ?? config.auth;
    const hasSignIn = !!auth && auth.mode !== "anonymous";
    const tokenProtected = !!config.token_configured || !!config.token;
    return !hasSignIn && !tokenProtected && config.anonymous !== false;
  }
  if (channel.channel_type === "fcp") {
    const config = channel.channel_config as FcpChannelConfig;
    const tokenProtected = !!config.token_configured || !!config.token;
    return !tokenProtected && config.anonymous !== false;
  }
  // Every other transport authenticates by construction: Slack signs its
  // requests, webhook, api and api_endpoint carry a token or key, A2A carries an
  // API key, a voice channel is called with an org API key or a member's
  // session, and a schedule has no inbound caller at all.
  return false;
}

export interface OrgExposure {
  channel: AgentChannel;
  agent: Agent | undefined;
  state: ExposureState;
  /// Configured to accept callers with no credential, whatever its current
  /// state. A suspended anonymous channel is still anonymous — resuming its
  /// agent opens it — so the row has to say so rather than reading
  /// "authenticated" until the moment it goes live.
  anonymous: boolean;
  /// Anonymous *and* currently live: an open door right now. This is what the
  /// severity sort and the visual weight key on; `anonymous` alone is a
  /// configuration to review.
  publiclyReachable: boolean;
  isTrigger: boolean;
  lastInvokedAt: string | null;
}

/// Every exposure in the org, across every agent.
///
/// The Agent detail page cannot answer "what is reachable from outside right
/// now" by construction — it shows one agent. This is the surface that can.
///
/// Channels are read per agent from `/v1/agents/{id}/channels`, and only for
/// agents whose list summary says they have one. Apps are archival and no
/// longer carry an agent's channels, so they cannot be the source here. The
/// per-agent query keys are the ones the channel mutations invalidate, so a
/// publish or unpublish shows up without a reload.
export function useOrgExposures() {
  const { data: agents, isLoading: agentsLoading } = useAgents({ includeArchived: true });
  const withChannels = useMemo(
    () => (agents ?? []).filter((agent) => (agent.channels ?? []).length > 0),
    [agents],
  );
  // `combine` keeps the result referentially stable while the underlying data is.
  const { channelsByAgent, channelsLoading } = useQueries({
    queries: withChannels.map((agent) => ({
      queryKey: queryKeys.agentChannels.list(agent.id),
      queryFn: () => listAgentChannels(agent.id),
    })),
    combine: (results) => ({
      channelsByAgent: results.map((result) => result.data),
      channelsLoading: results.some((result) => result.isLoading),
    }),
  });

  const exposures = useMemo<OrgExposure[]>(
    () =>
      withChannels.flatMap((agent, index) =>
        (channelsByAgent[index] ?? []).map((channel) => {
          const state = resolveExposureState(channel, agent);
          const anonymous = isAnonymousExposure(channel);
          return {
            channel,
            agent,
            state,
            anonymous,
            publiclyReachable: state === "live" && anonymous,
            isTrigger: isTriggerChannel(channel),
            lastInvokedAt: channel.last_invoked_at ?? null,
          };
        }),
      ),
    [withChannels, channelsByAgent],
  );

  return { exposures, isLoading: agentsLoading || channelsLoading };
}
