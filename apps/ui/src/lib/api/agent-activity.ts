import { api } from "./client";
import type { AgentActivityOverview } from "./types";

export async function getAgentActivity(): Promise<AgentActivityOverview> {
  const response = await api.get<AgentActivityOverview>("/v1/agents/activity");
  return response.data;
}
