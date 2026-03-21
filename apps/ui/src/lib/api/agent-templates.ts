// Agent Templates API — read-only templates installable as real Agents

import { api } from "./client";
import type { Agent, AgentTemplate } from "./types";

export async function listAgentTemplates(): Promise<AgentTemplate[]> {
  const response = await api.get<AgentTemplate[]>("/v1/agent-templates");
  return response.data;
}

export async function installAgentTemplate(slug: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agent-templates/${slug}/install`);
  return response.data;
}
