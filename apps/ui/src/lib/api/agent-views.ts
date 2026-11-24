import type { AgentViewLayout, PublishedAgentView } from "@everruns/generative-ui";
import { api } from "./client";

export interface AgentViewDefinition {
  title: string;
  layout: AgentViewLayout;
  allow_personalization: boolean;
  data_fields: string[];
  allow_agent_layout: boolean;
}

function sessionViewPath(sessionId: string, agentId?: string) {
  const path = `/v1/platform/sessions/${sessionId}/agent-view`;
  return agentId ? path + "?view_agent=" + encodeURIComponent(agentId) : path;
}

export async function getSessionAgentView(
  sessionId: string,
  agentId?: string,
): Promise<PublishedAgentView | null> {
  return (await api.get<PublishedAgentView | null>(sessionViewPath(sessionId, agentId))).data;
}

export async function saveSessionAgentView(
  sessionId: string,
  revision: number,
  layout: AgentViewLayout | null,
  agentId?: string,
): Promise<PublishedAgentView | null> {
  return (
    await api.put<PublishedAgentView | null>(sessionViewPath(sessionId, agentId), {
      revision,
      layout,
    })
  ).data;
}

export interface AgentViewSettings {
  revision: number;
  view: AgentViewDefinition | null;
}

export async function getAgentViewSettings(agentId: string): Promise<AgentViewSettings> {
  return (await api.get<AgentViewSettings>(`/v1/platform/agents/${agentId}/view`)).data;
}

export async function replaceAgentView(
  agentId: string,
  settings: AgentViewSettings,
): Promise<AgentViewSettings> {
  return (await api.put<AgentViewSettings>(`/v1/platform/agents/${agentId}/view`, settings)).data;
}
