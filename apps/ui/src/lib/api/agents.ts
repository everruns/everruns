import { apiClient } from "./client";
import type {
  Agent,
  CreateAgentRequest,
  UpdateAgentRequest,
} from "./types";

// Agent CRUD
export async function getAgents(): Promise<Agent[]> {
  return apiClient<Agent[]>("/v1/agents");
}

export async function getAgent(agentId: string): Promise<Agent> {
  return apiClient<Agent>(`/v1/agents/${agentId}`);
}

export async function createAgent(data: CreateAgentRequest): Promise<Agent> {
  return apiClient<Agent>("/v1/agents", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateAgent(
  agentId: string,
  data: UpdateAgentRequest
): Promise<Agent> {
  return apiClient<Agent>(`/v1/agents/${agentId}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}
