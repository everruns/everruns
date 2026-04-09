// Agent Examples API — read-only examples adoptable as real Agents

import { api } from "./client";
import type { Agent, AgentExample } from "./types";

export async function listAgentExamples(): Promise<AgentExample[]> {
  const response = await api.get<AgentExample[]>("/v1/agent-examples");
  return response.data;
}

export async function importAgentExample(name: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agents/import?from-example=${name}`);
  return response.data;
}
