// Agent Examples API — read-only examples adoptable as real Agents

import { api } from "./client";
import type { Agent, AgentExample } from "./types";

export async function listAgentExamples(): Promise<AgentExample[]> {
  const response = await api.get<AgentExample[]>("/v1/agent-examples");
  return response.data;
}

export async function adoptAgentExample(slug: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agent-examples/${slug}/use`);
  return response.data;
}
