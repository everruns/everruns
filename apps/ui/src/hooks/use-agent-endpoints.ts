"use client";

import { useMemo } from "react";
import { useApps } from "./use-apps";
import type { AppChannel } from "@/lib/api/types";

export interface AgentEndpoint {
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

export function useAgentEndpoints(agentId: string | undefined) {
  const { data: apps, isLoading, error } = useApps();

  const endpoints = useMemo<AgentEndpoint[]>(() => {
    if (!agentId || !apps) return [];
    return apps
      .filter((app) => app.agent_id === agentId)
      .flatMap((app) => app.channels.map((channel) => ({ channel })));
  }, [agentId, apps]);

  return { endpoints, isLoading, error };
}
