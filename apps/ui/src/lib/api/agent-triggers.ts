import { api } from "./client";
import type {
  AgentTrigger,
  AgentTriggerRun,
  CreateAgentTriggerRequest,
  TriggerAgentTriggerOutput,
  UpdateAgentTriggerRequest,
} from "./types";

// Temporary: GitHub trigger fields are not in the generated OpenAPI types yet.
// Remove once the types are regenerated.
export type GitHubTriggerFields = {
  trigger_type?: string;
  github_events?: string[];
  repositories?: string[];
  session_mode?: string;
};

const base = (agentId: string) => `/v1/agents/${agentId}/triggers`;

export async function listAgentTriggers(agentId: string): Promise<AgentTrigger[]> {
  return (await api.get<AgentTrigger[]>(base(agentId))).data;
}

export async function createAgentTrigger(
  agentId: string,
  request: Omit<CreateAgentTriggerRequest, "trigger_type" | "session_mode"> & GitHubTriggerFields,
): Promise<AgentTrigger> {
  return (await api.post<AgentTrigger>(base(agentId), request)).data;
}

export async function updateAgentTrigger(
  agentId: string,
  triggerId: string,
  request: Omit<UpdateAgentTriggerRequest, "session_mode"> & GitHubTriggerFields,
): Promise<AgentTrigger> {
  return (await api.patch<AgentTrigger>(`${base(agentId)}/${triggerId}`, request)).data;
}

export async function deleteAgentTrigger(agentId: string, triggerId: string): Promise<void> {
  await api.delete(`${base(agentId)}/${triggerId}`);
}

export async function runAgentTrigger(
  agentId: string,
  triggerId: string,
): Promise<TriggerAgentTriggerOutput> {
  return (await api.post<TriggerAgentTriggerOutput>(`${base(agentId)}/${triggerId}/trigger`)).data;
}

export async function listAgentTriggerRuns(
  agentId: string,
  triggerId: string,
): Promise<AgentTriggerRun[]> {
  return (await api.get<AgentTriggerRun[]>(`${base(agentId)}/${triggerId}/runs`)).data;
}
