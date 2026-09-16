"use client";

import { useMemo } from "react";
import { useApps } from "./use-apps";
import type { App, AppChannel } from "@/lib/api/types";

/// An endpoint together with the App that still owns its row.
///
/// Endpoints belong to the agent (EVE-1003), but the management API is still
/// App-scoped: there is no `GET /v1/agents/{id}/endpoints`, and EVE-1009 adds
/// no API. So the owning App travels with each endpoint, because every write —
/// configure, publish, run now — addresses it as `/v1/apps/{app_id}/channels/…`.
/// When the App domain is deleted (EVE-1011) this becomes a plain list and the
/// `app` field goes away with it.
export interface AgentEndpoint {
  channel: AppChannel;
  app: App;
}

/// Which channel types are exposures (a door traffic arrives through) versus
/// triggers (something that fires on its own). The Integrations tab shows both,
/// in separate sections, because they answer different questions: "how do I
/// reach this agent" and "when does it wake up on its own".
const TRIGGER_CHANNEL_TYPES = new Set(["schedule"]);

export function isTriggerChannel(channel: AppChannel): boolean {
  return TRIGGER_CHANNEL_TYPES.has(channel.channel_type);
}

/// Every endpoint of every App that points at this agent.
///
/// Apps are listed rather than fetched per id because an agent may be exposed
/// through several of them — one App per channel bundle was the old shape, and
/// nothing consolidated them. Archived Apps are excluded: their endpoints are
/// not reachable and showing them would imply the agent is exposed when it is
/// not.
export function useAgentEndpoints(agentId: string | undefined) {
  const { data: apps, isLoading, error } = useApps();

  const endpoints = useMemo<AgentEndpoint[]>(() => {
    if (!agentId || !apps) return [];
    return apps
      .filter((app) => app.agent_id === agentId)
      .flatMap((app) => app.channels.map((channel) => ({ channel, app })));
  }, [agentId, apps]);

  return { endpoints, isLoading, error };
}
